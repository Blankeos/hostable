default:
    just --list

dev *args:
    cargo run -p hostable -- {{ args }}

daemon *args:
    cargo run -p hostabled -- {{ args }}

fmt:
    cargo fmt

lint:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace --locked

web:
    bun install
    bun run build:web

docs-dev:
    bunx gittydocs@latest dev ./docs

docs-build:
    bunx gittydocs@latest build ./docs --out /tmp/hostable-docs-dist

[doc('Build release archives locally (needs cargo-dist, zig, cargo-zigbuild)')]
dist-build *args:
    bun install
    bun run build:admin
    mkdir -p target/web-admin
    cp -R web/admin/dist/. target/web-admin/
    dist build --artifacts=local {{ args }}
    sh scripts/bundle-release-artifacts.sh

[doc('Release: bump versions, commit, and tag from main (just tag [patch|minor|major])')]
tag bump="":
    sh scripts/tag_and_release.sh {{ bump }}
