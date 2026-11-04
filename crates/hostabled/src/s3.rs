use anyhow::{Context, Result, bail};
use hostable_core::S3Config;
use reqwest::{
    Url,
    header::{AUTHORIZATION, HOST, HeaderMap, HeaderValue},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    cmp::Reverse,
    env, fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone)]
pub struct S3Client {
    config: S3Config,
    access_key: String,
    secret_key: String,
    region: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize)]
pub struct S3PutResult {
    pub key: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct S3DeleteResult {
    pub key: String,
}

impl S3Client {
    pub fn from_env(config: S3Config) -> Result<Self> {
        let access_key = env::var(&config.access_key_env)
            .with_context(|| format!("missing env var {}", config.access_key_env))?;
        let secret_key = env::var(&config.secret_key_env)
            .with_context(|| format!("missing env var {}", config.secret_key_env))?;
        let region = config
            .region
            .clone()
            .filter(|region| !region.trim().is_empty())
            .unwrap_or_else(|| "us-east-1".to_string());
        Ok(Self {
            config,
            access_key,
            secret_key,
            region,
            http: reqwest::Client::new(),
        })
    }

    pub async fn test_connection(&self) -> Result<S3PutResult> {
        self.put_bytes(
            ".hostable-connection-test",
            b"ok".to_vec(),
            "text/plain; charset=utf-8",
        )
        .await
    }

    pub async fn put_file(
        &self,
        key: &str,
        path: &Path,
        content_type: &str,
    ) -> Result<S3PutResult> {
        let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        self.put_bytes(key, bytes, content_type).await
    }

    async fn put_bytes(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<S3PutResult> {
        let key = object_key(&self.config.prefix, key);
        let url = self.object_url(&key)?;
        let payload_hash = hex_sha256(&bytes);
        let headers = self.signed_headers("PUT", &url, &payload_hash, SystemTime::now())?;
        let response = self
            .http
            .put(url)
            .headers(headers)
            .header("content-type", content_type)
            .body(bytes.clone())
            .send()
            .await?
            .error_for_status()?;

        drop(response);
        Ok(S3PutResult {
            key,
            bytes: bytes.len() as u64,
        })
    }

    pub async fn prune_backup_sets(
        &self,
        server_id: &str,
        keep_last: u32,
    ) -> Result<Vec<S3DeleteResult>> {
        let prefix = object_key(&self.config.prefix, &format!("{server_id}/"));
        let keys = self.list_keys(&prefix).await?;
        let mut sets = Vec::<RemoteBackupSet>::new();
        for key in keys {
            let Some((timestamp, kind)) = parse_backup_key(server_id, &key) else {
                continue;
            };
            if let Some(set) = sets.iter_mut().find(|set| set.timestamp == timestamp) {
                match kind {
                    BackupFileKind::Archive => set.archive_key = Some(key),
                    BackupFileKind::Manifest => set.manifest_key = Some(key),
                }
            } else {
                let mut set = RemoteBackupSet {
                    timestamp,
                    ..RemoteBackupSet::default()
                };
                match kind {
                    BackupFileKind::Archive => set.archive_key = Some(key),
                    BackupFileKind::Manifest => set.manifest_key = Some(key),
                }
                sets.push(set);
            }
        }

        sets.sort_by_key(|set| Reverse(set.timestamp));
        let mut deleted = Vec::new();
        for set in sets.into_iter().skip(keep_last as usize) {
            for key in [set.archive_key, set.manifest_key].into_iter().flatten() {
                deleted.push(self.delete_key(&key).await?);
            }
        }
        Ok(deleted)
    }

    async fn list_keys(&self, prefix: &str) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        let mut continuation_token = None;
        loop {
            let url = self.list_url(prefix, continuation_token.as_deref())?;
            let payload_hash = hex_sha256(b"");
            let headers = self.signed_headers("GET", &url, &payload_hash, SystemTime::now())?;
            let body = self
                .http
                .get(url)
                .headers(headers)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            keys.extend(extract_xml_tags(&body, "Key"));
            if !parse_xml_bool(&body, "IsTruncated") {
                break;
            }
            continuation_token = extract_xml_tags(&body, "NextContinuationToken")
                .into_iter()
                .next();
            if continuation_token.is_none() {
                break;
            }
        }
        Ok(keys)
    }

    async fn delete_key(&self, key: &str) -> Result<S3DeleteResult> {
        let url = self.object_url(key)?;
        let payload_hash = hex_sha256(b"");
        let headers = self.signed_headers("DELETE", &url, &payload_hash, SystemTime::now())?;
        self.http
            .delete(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?;
        Ok(S3DeleteResult {
            key: key.to_string(),
        })
    }

    fn object_url(&self, key: &str) -> Result<Url> {
        let endpoint = self
            .config
            .endpoint_url
            .clone()
            .filter(|endpoint| !endpoint.trim().is_empty())
            .unwrap_or_else(|| format!("https://s3.{}.amazonaws.com", self.region));
        let url = format!(
            "{}/{}/{}",
            endpoint.trim_end_matches('/'),
            uri_encode(&self.config.bucket, false),
            uri_encode(key, true)
        );
        Url::parse(&url).with_context(|| format!("invalid S3 URL {url}"))
    }

    fn list_url(&self, prefix: &str, continuation_token: Option<&str>) -> Result<Url> {
        let endpoint = self
            .config
            .endpoint_url
            .clone()
            .filter(|endpoint| !endpoint.trim().is_empty())
            .unwrap_or_else(|| format!("https://s3.{}.amazonaws.com", self.region));
        let mut query = String::new();
        if let Some(token) = continuation_token {
            query.push_str("continuation-token=");
            query.push_str(&uri_encode(token, false));
            query.push('&');
        }
        query.push_str("list-type=2&prefix=");
        query.push_str(&uri_encode(prefix, false));
        let url = format!(
            "{}/{}?{}",
            endpoint.trim_end_matches('/'),
            uri_encode(&self.config.bucket, false),
            query
        );
        Url::parse(&url).with_context(|| format!("invalid S3 list URL {url}"))
    }

    fn signed_headers(
        &self,
        method: &str,
        url: &Url,
        payload_hash: &str,
        now: SystemTime,
    ) -> Result<HeaderMap> {
        let host = url.host_str().context("S3 URL is missing a host")?;
        let host = match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };
        let (date, amz_date) = amz_dates(now)?;
        let credential_scope = format!("{date}/{}/s3/aws4_request", self.region);
        let canonical_uri = url.path();
        let canonical_query = url.query().unwrap_or("");
        let canonical_headers =
            format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n");
        let signed_headers = "host;x-amz-content-sha256;x-amz-date";
        let canonical_request = format!(
            "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            hex_sha256(canonical_request.as_bytes())
        );
        let signature = hex(signing_key(
            &self.secret_key,
            &date,
            &self.region,
            "s3",
            "aws4_request",
            &string_to_sign,
        ));
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.access_key
        );

        let mut headers = HeaderMap::new();
        headers.insert(HOST, HeaderValue::from_str(&host)?);
        headers.insert("x-amz-content-sha256", HeaderValue::from_str(payload_hash)?);
        headers.insert("x-amz-date", HeaderValue::from_str(&amz_date)?);
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&authorization)?);
        Ok(headers)
    }
}

#[derive(Debug, Clone, Default)]
struct RemoteBackupSet {
    timestamp: u64,
    archive_key: Option<String>,
    manifest_key: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum BackupFileKind {
    Archive,
    Manifest,
}

fn parse_backup_key(server_id: &str, key: &str) -> Option<(u64, BackupFileKind)> {
    let name = key.rsplit('/').next()?;
    let suffix = name.strip_prefix(&format!("{server_id}-"))?;
    if let Some(timestamp) = suffix.strip_suffix(".tar.zst") {
        return timestamp
            .parse()
            .ok()
            .map(|value| (value, BackupFileKind::Archive));
    }
    if let Some(timestamp) = suffix.strip_suffix(".manifest.json") {
        return timestamp
            .parse()
            .ok()
            .map(|value| (value, BackupFileKind::Manifest));
    }
    None
}

fn extract_xml_tags(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut rest = xml;
    let mut values = Vec::new();
    while let Some(start) = rest.find(&open) {
        let value_start = start + open.len();
        let Some(end) = rest[value_start..].find(&close) else {
            break;
        };
        let raw = &rest[value_start..value_start + end];
        values.push(unescape_xml(raw));
        rest = &rest[value_start + end + close.len()..];
    }
    values
}

fn parse_xml_bool(xml: &str, tag: &str) -> bool {
    extract_xml_tags(xml, tag)
        .into_iter()
        .next()
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn unescape_xml(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn object_key(prefix: &str, key: &str) -> String {
    let prefix = prefix.trim_matches('/');
    let key = key.trim_start_matches('/');
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}/{key}")
    }
}

fn signing_key(
    secret_key: &str,
    date: &str,
    region: &str,
    service: &str,
    terminal: &str,
    string_to_sign: &str,
) -> [u8; 32] {
    let date_key = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), date.as_bytes());
    let region_key = hmac_sha256(&date_key, region.as_bytes());
    let service_key = hmac_sha256(&region_key, service.as_bytes());
    let signing_key = hmac_sha256(&service_key, terminal.as_bytes());
    hmac_sha256(&signing_key, string_to_sign.as_bytes())
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        key_block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut outer = [0x5c; BLOCK_SIZE];
    let mut inner = [0x36; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        outer[i] ^= key_block[i];
        inner[i] ^= key_block[i];
    }

    let mut inner_hash = Sha256::new();
    inner_hash.update(inner);
    inner_hash.update(message);
    let inner_hash = inner_hash.finalize();

    let mut outer_hash = Sha256::new();
    outer_hash.update(outer);
    outer_hash.update(inner_hash);
    outer_hash.finalize().into()
}

fn hex_sha256(bytes: impl AsRef<[u8]>) -> String {
    hex(Sha256::digest(bytes.as_ref()))
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn uri_encode(input: &str, preserve_slash: bool) -> String {
    let mut out = String::new();
    for byte in input.bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (preserve_slash && byte == b'/');
        if keep {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&hex([byte]).to_uppercase());
        }
    }
    out
}

fn amz_dates(now: SystemTime) -> Result<(String, String)> {
    let duration = now
        .duration_since(UNIX_EPOCH)
        .context("system clock before unix epoch")?;
    let seconds = duration.as_secs() as i64;
    let days = seconds / 86_400;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days)?;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let date = format!("{year:04}{month:02}{day:02}");
    let amz_date = format!("{date}T{hour:02}{minute:02}{second:02}Z");
    Ok((date, amz_date))
}

fn civil_from_days(days_since_epoch: i64) -> Result<(i64, i64, i64)> {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_piece = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_piece + 2) / 5 + 1;
    let month = month_piece + if month_piece < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    if year < 1970 {
        bail!("dates before 1970 are not supported");
    }
    Ok((year, month, day))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn formats_amz_dates() {
        let (_, amz_date) =
            amz_dates(UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)).unwrap();
        assert_eq!(amz_date, "20231114T221320Z");
    }

    #[test]
    fn encodes_s3_keys() {
        assert_eq!(
            uri_encode("backups/friends world.tar.zst", true),
            "backups/friends%20world.tar.zst"
        );
    }

    #[tokio::test]
    async fn uploads_to_s3_compatible_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut stream).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .await
                .unwrap();
            request
        });

        let client = S3Client {
            config: S3Config {
                bucket: "bucket".to_string(),
                region: Some("us-east-1".to_string()),
                endpoint_url: Some(format!("http://{addr}")),
                prefix: "hostable".to_string(),
                access_key_env: "HOSTABLE_TEST_ACCESS".to_string(),
                secret_key_env: "HOSTABLE_TEST_SECRET".to_string(),
            },
            access_key: "access".to_string(),
            secret_key: "secret".to_string(),
            region: "us-east-1".to_string(),
            http: reqwest::Client::new(),
        };

        let result = client
            .put_bytes(
                "smoke/archive.tar.zst",
                b"backup".to_vec(),
                "application/zstd",
            )
            .await
            .unwrap();
        let request = server.await.unwrap();
        let lower = request.to_ascii_lowercase();

        assert_eq!(result.key, "hostable/smoke/archive.tar.zst");
        assert_eq!(result.bytes, 6);
        assert!(request.starts_with("PUT /bucket/hostable/smoke/archive.tar.zst HTTP/1.1\r\n"));
        assert!(lower.contains("authorization: aws4-hmac-sha256 credential=access/"));
        assert!(lower.contains("x-amz-content-sha256:"));
        assert!(lower.contains("content-type: application/zstd"));
        assert!(request.ends_with("backup"));
    }

    #[tokio::test]
    async fn prunes_remote_backup_sets_by_retention_count() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let responses = [
                concat!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/xml\r\nconnection: close\r\n\r\n",
                    "<ListBucketResult>",
                    "<IsTruncated>false</IsTruncated>",
                    "<Contents><Key>hostable/friends/friends-1.tar.zst</Key></Contents>",
                    "<Contents><Key>hostable/friends/friends-1.manifest.json</Key></Contents>",
                    "<Contents><Key>hostable/friends/friends-2.tar.zst</Key></Contents>",
                    "<Contents><Key>hostable/friends/friends-2.manifest.json</Key></Contents>",
                    "<Contents><Key>hostable/friends/friends-3.tar.zst</Key></Contents>",
                    "<Contents><Key>hostable/friends/friends-3.manifest.json</Key></Contents>",
                    "</ListBucketResult>"
                )
                .as_bytes()
                .to_vec(),
                b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    .to_vec(),
                b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    .to_vec(),
            ];
            let mut requests = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                requests.push(read_http_request(&mut stream).await);
                stream.write_all(&response).await.unwrap();
            }
            requests
        });

        let client = S3Client {
            config: S3Config {
                bucket: "bucket".to_string(),
                region: Some("us-east-1".to_string()),
                endpoint_url: Some(format!("http://{addr}")),
                prefix: "hostable".to_string(),
                access_key_env: "HOSTABLE_TEST_ACCESS".to_string(),
                secret_key_env: "HOSTABLE_TEST_SECRET".to_string(),
            },
            access_key: "access".to_string(),
            secret_key: "secret".to_string(),
            region: "us-east-1".to_string(),
            http: reqwest::Client::new(),
        };

        let deleted = client.prune_backup_sets("friends", 2).await.unwrap();
        let requests = server.await.unwrap();

        assert_eq!(
            deleted
                .iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>(),
            vec![
                "hostable/friends/friends-1.tar.zst",
                "hostable/friends/friends-1.manifest.json",
            ]
        );
        assert!(
            requests[0]
                .starts_with("GET /bucket?list-type=2&prefix=hostable%2Ffriends%2F HTTP/1.1\r\n")
        );
        assert!(
            requests[1]
                .starts_with("DELETE /bucket/hostable/friends/friends-1.tar.zst HTTP/1.1\r\n")
        );
        assert!(
            requests[2].starts_with(
                "DELETE /bucket/hostable/friends/friends-1.manifest.json HTTP/1.1\r\n"
            )
        );
    }

    async fn read_http_request(stream: &mut tokio::net::TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let read = stream.read(&mut chunk).await.unwrap();
            assert!(read > 0, "connection closed before request finished");
            buffer.extend_from_slice(&chunk[..read]);
            if complete_http_request(&buffer) {
                return String::from_utf8(buffer).unwrap();
            }
        }
    }

    fn complete_http_request(buffer: &[u8]) -> bool {
        let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let headers = String::from_utf8_lossy(&buffer[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")?
                    .trim()
                    .parse::<usize>()
                    .ok()
            })
            .unwrap_or(0);
        buffer.len() >= header_end + 4 + content_length
    }
}
