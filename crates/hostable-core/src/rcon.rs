use crate::{HostableError, Result};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

const SERVERDATA_AUTH: i32 = 3;
const SERVERDATA_AUTH_RESPONSE: i32 = 2;
const SERVERDATA_EXECCOMMAND: i32 = 2;
const SERVERDATA_RESPONSE_VALUE: i32 = 0;

pub fn send_command(addr: &str, password: &str, command: &str) -> Result<String> {
    let mut stream = TcpStream::connect(addr).map_err(|error| {
        HostableError::InvalidInput(format!(
            "cannot reach the server console at {addr} ({error}). The server may be stopped, still starting, or have RCON disabled."
        ))
    })?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    write_packet(&mut stream, 1, SERVERDATA_AUTH, password)?;
    // Source-engine servers (ARK, Palworld) send an empty RESPONSE_VALUE packet
    // before the real auth response, so skip until we see the auth response.
    let mut auth = read_packet(&mut stream)?;
    while auth.kind == SERVERDATA_RESPONSE_VALUE && auth.request_id != -1 {
        auth = read_packet(&mut stream)?;
    }
    if auth.request_id == -1 || auth.kind != SERVERDATA_AUTH_RESPONSE {
        return Err(HostableError::InvalidInput(
            "RCON authentication failed".to_string(),
        ));
    }

    write_packet(&mut stream, 2, SERVERDATA_EXECCOMMAND, command)?;
    let response = read_packet(&mut stream)?;
    if response.kind != SERVERDATA_RESPONSE_VALUE && response.kind != SERVERDATA_EXECCOMMAND {
        return Err(HostableError::InvalidInput(
            "RCON returned an unexpected response".to_string(),
        ));
    }
    Ok(response.body)
}

#[derive(Debug)]
struct RconPacket {
    request_id: i32,
    kind: i32,
    body: String,
}

fn write_packet(stream: &mut TcpStream, request_id: i32, kind: i32, body: &str) -> Result<()> {
    let body = body.as_bytes();
    let length = 4 + 4 + body.len() as i32 + 2;
    // Send the whole packet in one write: Minecraft's RCON handler does a single
    // read() per packet and drops the connection if the packet arrives in pieces.
    let mut packet = Vec::with_capacity(4 + length as usize);
    packet.extend_from_slice(&length.to_le_bytes());
    packet.extend_from_slice(&request_id.to_le_bytes());
    packet.extend_from_slice(&kind.to_le_bytes());
    packet.extend_from_slice(body);
    packet.extend_from_slice(&[0, 0]);
    stream.write_all(&packet)?;
    Ok(())
}

fn read_packet(stream: &mut TcpStream) -> Result<RconPacket> {
    let mut length_bytes = [0u8; 4];
    stream.read_exact(&mut length_bytes)?;
    let length = i32::from_le_bytes(length_bytes);
    if !(10..=8192).contains(&length) {
        return Err(HostableError::InvalidInput(format!(
            "invalid RCON packet length: {length}"
        )));
    }

    let mut data = vec![0u8; length as usize];
    stream.read_exact(&mut data)?;
    let request_id = i32::from_le_bytes(data[0..4].try_into().expect("packet has id"));
    let kind = i32::from_le_bytes(data[4..8].try_into().expect("packet has kind"));
    let body = String::from_utf8_lossy(&data[8..data.len().saturating_sub(2)]).to_string();
    Ok(RconPacket {
        request_id,
        kind,
        body,
    })
}
