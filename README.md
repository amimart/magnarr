# Magnarr

![Status](https://img.shields.io/badge/status-🚧%20WIP-yellow?style=for-the-badge)
[![Codecov](https://img.shields.io/codecov/c/github/amimart/magnarr?style=for-the-badge)](https://codecov.io/github/amimart/magnarr)
[![lint](https://img.shields.io/github/actions/workflow/status/amimart/magnarr/lint.yaml?label=lint&style=for-the-badge&logo=github)](https://github.com/amimart/magnarr/actions/workflows/lint.yaml)
[![build](https://img.shields.io/github/actions/workflow/status/amimart/magnarr/build.yaml?label=build&style=for-the-badge&logo=github)](https://github.com/amimart/magnarr/actions/workflows/build.yaml)
[![test](https://img.shields.io/github/actions/workflow/status/amimart/magnarr/test.yaml?label=test&style=for-the-badge&logo=github)](https://github.com/amimart/magnarr/actions/workflows/test.yaml)

Magnarr is a lightweight magnet-based download orchestrator.

It focuses only on:

- ingesting magnet links
- tracking torrent downloads
- importing completed files

>🚧 **WARNING**: Magnarr is not mature enough to be considered production-grade. Its API may change without notice. But feedbacks are welcomed 😉

## Install

### Build from source

Building Magnarr requires Git and the Rust toolchain:

```shell
git clone https://github.com/amimart/magnarr.git
cd magnarr
cargo build --release --locked
```

The resulting binary is available at `target/release/magnarr`. It can also be
installed in Cargo's binary directory:

```shell
cargo install --path . --locked
```

### Docker

Published releases are available from the GitHub Container Registry:

```shell
docker pull ghcr.io/amimart/magnarr:latest
```

## Quickstart

The repository includes a Docker Compose setup intended as a complete local
development environment. It builds Magnarr from the current source tree and
starts it alongside a preconfigured qBittorrent instance.

You need Docker with Docker Compose support and `make`. Then run:

```shell
git clone https://github.com/amimart/magnarr.git
cd magnarr
make local-init
make local-start
```

The initialization command creates the local deployment directories under
`target/deploy/local` and installs the development configuration files. The
Compose environment then:

- builds and starts Magnarr;
- starts qBittorrent with its Web UI and torrent port exposed;
- shares the same download directory between both services;
- provides a media directory where Magnarr can import completed downloads;
- persists Magnarr data and qBittorrent configuration on the host.

Once the services are running, the following interfaces are available:

- Magnarr GraphiQL interface: <http://localhost:9393/graphql>
- qBittorrent Web UI: <http://localhost:8080>

The local qBittorrent credentials are `admin` / `adminadmin`.

To inspect or stop the environment:

```shell
docker compose logs -f
make local-stop
```

This setup can be used as a starting point for a real deployment. Before doing
so, use the published Magnarr image, pin image versions, replace the development
credentials, choose persistent host paths, and protect the exposed interfaces.
Magnarr and qBittorrent must retain access to the same download directory.

## Configuration

Magnarr keeps its configuration and internal data in a home directory. It uses
`~/.magnarr` by default; another location can be selected with `--home` or the
`MAGNARR_HOME` environment variable:

```shell
magnarr --home /srv/magnarr init
```

The `init` command creates the home directory, writes a default `config.yaml`,
and initializes the Redb database. It expects the selected home directory not
to exist yet, to avoid overwriting an existing installation.

Magnarr reads `<home>/config.yaml` when it starts. Relative paths in the
configuration, including the download directory and database path, are resolved
from the home directory. Absolute paths are used unchanged.

```yaml
app:
  poll_interval: 30s
  download_dir: ./download
server:
  listen_addr: localhost:9393
  max_page_size: 100
store:
  path: ./data/magnarr.redb
torrent_client:
  qbittorrent:
    host: http://localhost:8080
    username: admin
    password: adminadmin
```

The main settings are:

- `app.poll_interval`: how often torrent states are refreshed;
- `app.download_dir`: where qBittorrent stores completed downloads;
- `server.listen_addr`: address used by the GraphQL HTTP server;
- `server.max_page_size`: maximum number of downloads returned per page;
- `store.path`: location of the Redb database;
- `torrent_client.qbittorrent`: qBittorrent URL and credentials.

Configuration is applied in this order, with later sources taking precedence:
compiled defaults, `config.yaml`, `MAGNARR_*` environment variables, then
options passed to `magnarr start`. Run `magnarr start --help` to see the
available command-line overrides.
