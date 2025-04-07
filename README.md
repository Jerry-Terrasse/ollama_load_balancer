# Ollama Load Balancer

An autonomous Rust utility that load balances multiple Ollama servers. It optimizes response times and reliability by dispatching requests to the most suitable server in parallel, while maintaining a robust health-value system.

## Features

- **Load Balancing:** Distributes requests randomly among suitable backend servers.
- **High Performance:** For chat requests, parallel requests are sent to several backends chosen by their health scores, with the fastest response stream returned to the user.
- **Health Value System:** Each server's health score dynamically increases on successful completion, decreases on error responses, and gets an extra boost for delivering the fastest response.
- **High Availability:** The load balancer works fine even if one or more servers are down, leveraging redundant backends and only failing when all selected backends are unavailable.

## Installation

<!-- ### Executable
Download the latest release executable from GitHub.

### Nix (MacOS, Linux)

### Docker -->

### Build from Source

```shell
git clone https://github.com/Jerry-Terrasse/ollama_load_balancer.git
cd ollama_load_balancer
cargo build --release
```

## Usage

You can specify Ollama backend servers by cli arguments or by using a server file.

For example:

```shell
ollama_load_balancer -l 0.0.0.0:11434 \
  --servers "http://192.168.1.100:11434=s0" \
  --servers "http://192.168.1.101:11434=s1"

ollama_load_balancer -l 0.0.0.0:11434 --server-file server_list.txt
```

### Specifying Backend Servers

Each server is specified in the format `http://<ip>:<port>=<name>`, where `<name>` is a human-readable identifier for the server.

The `--server-file` option allows you to specify a file containing a list of servers, one per line:

```text
http://192.168.1.100:11434=s0
http://192.168.1.101:11434=s1
```

### Other Options

| Option | Alias | Description | Default |
|---|---|---|---|
|`--listen`|`-l`|Listening address and port for the load balancer.|`0.0.0.0:11434`|
|`--timeout`| - |Timeout for common requests in seconds. (except for `/api/chat`)|1|
|`--timeout-ft`| - |Maximum time in seconds to wait for a server to return the first token.|10|
|`--time-measure`| - |Maximum time in seconds to wait for a server to return the last token.|2|

## API Endpoints

### Compatible with Ollama

| Endpoint | Description | Forward Type |
|---|---|---|
|`/`|Returns with `200 OK` for health check.|Not forwarded|
|`/api/tags`|Returns an aggregate of all available models from all the backends.|Not forwarded|
|`/api/show`|Returns model information fetched from suitable backends.|Sequentially forwarded|
|`/api/generate`|(Partially supported) Returns `200 OK` to make `ollama` cli happy.|Not forwarded|
|`/api/chat`|Returns the stream of the fastest server.|Parallelly forwarded|

### Load Balancer Specific

These endpoints are specific to the load balancer and are not part of the standard Ollama API.

| Endpoint | Description |
|---|---|
|`/status`|Returns the status of all servers in the load balancer. (not implemented yet)|
|`/add_server`|Adds a new server to the load balancer. (not implemented yet)|
|`/sync_servers`|Synchronizes the server list with the load balancer. (not implemented yet)|

### TODO List

- [ ] Use GitHub actions to build and release
- [ ] Implement Ollama compatible endpoints
  - [ ] `/api/ps`
  - [ ] `/api/generate` (full support)
- [ ] Implement more API
  - [ ] `/status`
  - [ ] `/add_server`
  - [ ] `/sync_servers`
- [ ] Simple test on different platforms
  - [x] Linux
  - [ ] MacOS
  - [ ] Windows
- [ ] Fatest-Finish-First (F3) policy
- [ ] Try hacking some info into ollama cli for convenience
- [ ] Support authentication

## Release Notes

### 2.6 (WIP)

- feat: refactor timeout and performance mechanism, small requests are faster
- chore: add own README version
- chore: add TODO list
- chore: add release workflow with GitHub actions

### 2.5

- chore: improve error handling in handler.rs
- feat: filter out servers that do not have the requested model
- feat: support updating health value according to the performance

### 2.4

- feat: partially support `/api/generate` endpoint (enough for `ollama run`)
- feat: support `/api/show` endpoint with sequential high-availability
- chore: clean dead code
- chore: simplify code with `make_json_resp()`
- fix: logical bug in `select_servers()`

### 2.3

- feat: add server selection logic according to the health value
- feat: support `/api/tags` endpoint
- feat: refactor the logging method with `tracing` crate

### 2.2

- feat: add `--server-file` option to load server lists
- chore: split some functions to backend.rs
- feat: add `api_tags()` and `api_ps()` to fetch backend model status
- feat: add `sync_server()` to synchronize backend status
- feat: add server health-value mechanism

### 2.1

- chore: add request dispatching
- feat: add `--timeout-load`, `--t0` and `--t1` options for timeout settings
- feat: support performance measures

### 2.0

- feat: add `--listen` option to specify listening address
- chore: refactor the project structure
- feat: implement bytes counting
- feat: support parallel requests

- **Refactored Timeout & Performance Mechanism:** Small requests now receive faster responses.
- **Improved Health Value Logic:** Finalized server health calculations with a proper update cycle.
- **Enhanced Error Handling:** Additional error checks in the request handler and debugging through an `into_string` method for RepackedResponse.
- **Server Selection Updates:** Do not select servers that do not have the requested model and refine parallel backend requests.

New Endpoints Support:
- `/api/tags`: Fetch backend model tags.
- `/api/show`: High-availability endpoint for static request testing.
- Partial support for `/api/generate`.

Additional CLI Options:
- Support for `--server-file` to load server lists.
- `-l` option for specifying a custom listening address.
- Added timeout load options and performance measures (using t0, t1, etc.).

<details>
<summary>Releases from the original author</summary>

### 1.0.3
https://github.com/BigBIueWhale/ollama_load_balancer/blob/RLS_01_00_03_2025_01_28/release

**Changes:**
- Print activity status list of all servers every time something changes

- Breaking change- human-readable name must be specified in CLI arguments.

### 1.0.2
https://github.com/BigBIueWhale/ollama_load_balancer/blob/RLS_01_00_02_2024_11_17/release/ollama_load_balancer.exe

**Changes:**
- Fix: Support any HTTP request type in addition to `POST`. This includes `GET`, `POST`, `PUT`, `DELETE`, `HEAD`, `OPTIONS`, `PATCH`, `TRACE`.\
The https://openwebui.com/ server was trying to use `GET /api/tags` so this feature is required for openwebui servers to agree to use this load balancer.

### 1.0.1
https://github.com/BigBIueWhale/ollama_load_balancer/blob/RLS_01_00_01_2024_10_22/release/ollama_load_balancer.exe

**Changes:**
- Style: Avoid confusing print that includes green status `🟢 Server {} now available` when just after a failure.
- Logic: Fix premature demoting of `Unreliable` server to `SecondChanceGiven`- that would cause bug where if user cancels generation mid-stream, an `Unreliable` server would be marked as `SecondChanceGiven` despite no failure occurring.
- Logic: Fix bug where server gets marked as `Reliable` before stream ends and is successful- that would cause a server that fails every time mid-stream to ruin the user experience.
- Code: Refactor- Use server "ADDRESS:PORT" as key to data structure holding the state of all servers, instead of holding Arc reference to specific server, this avoids needing multiple locks, improves performance, and fixes logical race condition caused by multiple locks.
- Doc: Optimize documentation for end-users

### 1.0.0
https://github.com/BigBIueWhale/ollama_load_balancer/blob/RLS_01_00_00_2024_10_22/release/ollama_load_balancer.exe

**Features:**
- Standalone command-line executable for Windows 10/11 with app icon, linked with MSVC 64-bit toolchain
- Tested on `Windows 11 Pro 23H2`
- Source code is cross platform- compile works on Ubuntu 22.04
- Load balancing implemented
- Streaming HTTP POST request handled by utility
- Robust error handling- edge cases managed
- Well-documented
- Easy-to-read emoji logs to console
- Configurable timeout via command line argument
- Configurable Ollama servers `IP:PORT` list via command line arguments
- Stateless- no saved state between executable runs, no configuration files- all CLI
- Supports any REST server based on `HTTP POST` requests, not just Ollama.
- Optimized for immediate response to user- avoid user needing to wait
- Ideal server-ranking implementation for performance-identical Ollama servers in chaotic environment where they can be turned on and off on a whim.

</details>

## License

Distributed under the MIT License. See LICENSE for more information.