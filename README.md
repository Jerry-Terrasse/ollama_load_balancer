# Ollama Load Balancer
Rust utility that load balances multiple https://ollama.com/ servers

![project logo small](./doc/logo/logo_small.png)

## Release Notes
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

## Usage
1. Download the [latest release](#release-notes) executable

2. Run in Powershell, CMD, or terminal. Make sure to [allow access to both public and private networks](./doc/screenshots/allow_access_to_public_and_private_networks.png) during the first time running the utility.
```sh
C:\Users\user\Downloads>ollama_load_balancer.exe --version
ollama_load_balancer 1.0.1

C:\Users\user\Downloads>ollama_load_balancer.exe --server http://192.168.150.134:11434 --server http://192.168.150.135:11434 --server http://192.168.150.136:11434

📒 Ollama servers list:
1. http://192.168.150.134:11434
2. http://192.168.150.135:11434
3. http://192.168.150.136:11434

⚙️  Timeout setting: Will abandon Ollama server after 30 seconds of silence

👂 Ollama Load Balancer listening on http://0.0.0.0:11434

🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64198
🤖🦸 Chose reliable server: http://192.168.150.135:11434 to serve client 127.0.0.1:64200
🤖🦸 Chose reliable server: http://192.168.150.136:11434 to serve client 127.0.0.1:64203
🤷 No available servers to serve client 127.0.0.1:64210
🟢 Server http://192.168.150.134:11434 now available
🟢 Server http://192.168.150.136:11434 now available
🟢 Server http://192.168.150.135:11434 now available
☠️  Received CTRL+C, shutting down gracefully...

C:\Users\user\Downloads>
```

In this example, we set up 4 VS Code instances (to simulate users) and turn on 3 Ollama servers. We first quickly request LLM chat completion from all four users- Three manage, but the fourth causes: `🤷 No available servers to serve client`

## Purpose
A single Ollama server can (and should) only serve one request at the same time.

Hardware for an Ollama server is expensive. This load balancer allows to distribute a limited number of Ollama servers optimally to multiple users on a local network.

Let's say you have 60 users of an LLM service and 6 Ollama servers. What's the probability that 10% or more of your users are prompting the LLM at the same time?

## Principal of Operation
All users on the network configure their `continue.dev` (VS Code extension) to point to the IP address of this load balancer instead of manually choosing a specific Ollama server.

Any HTTP POST request for an LLM completion from a user triggers this utility to make an identical HTTP POST request to a real Ollama server on bahalf of the user. All while streaming the response back to the user.

We only choose servers that are currently available, we can know which Ollama servers are available based on the assumption that users only access the Ollama servers via this load balancer.

### Unreliable Servers

We assume that the list of Ollama servers isn't perfect.\
A servers might be temporarily or permanently off, a server might have changed its IP address.\
A server might be faulty- fail every time.

Therefore we introduced a state for each server: `failure_record: FailureRecord`

```rs
enum FailureRecord {
    Reliable,
    Unreliable,
    SecondChanceGiven,
}
```

We want to avoid a bad server from causing the user experience to be unreliable when using this load balancer.

Therefore if a server fails during a request, we mark it as `Unreliable`.\
We only choose an unreliable server to process a request if there's no `Reliable` server available (not `busy`)

If an `Unreliable` server is given a chance to repent, and it succeeds to process a request, then it's marked as `Reliable` again, because that most likely means that somebody turned the PC and the Ollama server on.

Question is: how do we choose from multiple possible `Unreliable` servers? How do we make sure that they all get a timely chance to repent?

That's what `SecondChanceGiven` is for. It's a state that we can flip to ensure that we cycle through all `Unreliable` servers evenly, avoiding the situation where we try a single `Unreliable` server twice to no avail while ignoring the other (possibly good) servers.

## Supported Usages
We support `continue.dev` and also any client that streams responses from an Ollama server such as https://openwebui.com/

We support `/api/chat`, `/api/generate` (CTRL+i in `continue.dev`), `/api/show`, and actually we support any POST request. We support streaming based on `Transfer-Encoding: chunked` and `Content-Type: application/x-ndjson`.

Static HTTP is also supported with `stream: false` in JSON given in POST request to Ollama.

## Streaming

The LLM doesn't have the complete response immediately which is why Ollama streams the completions.

Streaming is implemented using `Newline Delimited JSON format` (ndjson). See `Content-Type: application/x-ndjson`, although this format is not hard-coded in the load balancer.

Each line of the ndjson format is mapped to one object in a JSON array.

## Dependencies
These are the versions I used:

- cargo 1.82.0 (8f40fc59f 2024-08-21) on `Windows 11 Pro 23H2`

- Ollama version 0.3.13 on `Windows 10 Pro 22H2`

- VS Code version 1.90.2 on `Windows 11 Pro 23H2`

- `Continue - Codestral, Claude, and more` VS Code extension by `Continue` version 0.8.46 - 2024-08-11

- `rust-analyzer` v0.3.2146 by `The Rust Programming Language`

## Lab testing
1. Use a Windows host with at least 64 gigabytes of RAM and at least 8 CPU cores so that you can run [three virtual machines at the same time](./doc/screenshots/virtual_machines_running_ollama.png).

2. While the virtual machines are connected to the internet, install Ollama and run `ollama pull deepseek-coder:1.3b-instruct-q4_0`. Then kill Ollama from the Windows tray by right-clicking the tray icon. We choose this specific model because it has acceptable performance in CPU mode, and doesn't use much VRAM.

3. Set each virtual machine to be connected with a [host only network adapter](./doc/screenshots/virtual_machine_settings_host_only_network_adapter.png) so that the host (running the load balancer) has access to three Ollama servers on the local network. Now the VMs don't have world wide web internet access anymore.

4. Instead of running `ollama serve`, use [this batch file](https://github.com/BigBIueWhale/assistant_coder/blob/3cfa95ed35605e1d07fea4f8c479729eb0bfb9c9/run_ollama.bat) in each virtual machine so that Ollama runs on all network interfaces (`0.0.0.0`) instead of localhost.

5. [Find out the IP addresses](./doc/screenshots/virtual_machines_ip_addresses.png) of the virtual machines that VMWare decided to assign.\
**Adjust the server configuration** to point to the correct IP addresses of your Ollama servers.

6. Configure `continue.dev` (VS Code extension) to access the Ollama server at: `http://127.0.0.1:11434/` because in lab testing we're running the load balancer on the host- the same device running VS Code.

    The `continue.dev` VS Code extension config.json:
    ```json
    {
      "models": [
        {
          "title": "DeepSeek Coder",
          "provider": "ollama",
          "apiBase": "http://127.0.0.1:11434/",
          "model": "deepseek-coder:1.3b-instruct-q4_0",
          "contextLength": 4096
        }
      ],
      "tabAutocompleteOptions": {
        "disable": true
      },
      "completionOptions": {
        "maxTokens": 2048
      },
      "allowAnonymousTelemetry": false,
      "docs": []
    }
    ```
7. Open multiple instances of VS Code to prompt the LLM concurrently and test-out the load balancer.

## Edge Cases
### VM running Ollama crashes while generating- timeout 30 seconds (default)
```sh
C:\Users\user\Downloads>ollama_load_balancer.exe --version
ollama_load_balancer 1.0.1

C:\Users\user\Downloads>ollama_load_balancer.exe --server http://192.168.150.134:11434

📒 Ollama servers list:
1. http://192.168.150.134:11434

⚙️  Timeout setting: Will abandon Ollama server after 30 seconds of silence

👂 Ollama Load Balancer listening on http://0.0.0.0:11434

🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64266
⛔😱 Server http://192.168.150.134:11434 failed during streaming, now marked Unreliable. Error: error decoding response body
⚠️  Connection closed with Unreliable Server http://192.168.150.134:11434
☠️  Received CTRL+C, shutting down gracefully...

C:\Users\user\Downloads>
```

### PC running Ollama server was off (hard-coded 1 second initial-connection timeout)
```sh
C:\Users\user\Downloads>ollama_load_balancer.exe --version
ollama_load_balancer 1.0.1

C:\Users\user\Downloads>ollama_load_balancer.exe --server http://192.168.150.134:11434

📒 Ollama servers list:
1. http://192.168.150.134:11434

⚙️  Timeout setting: Will abandon Ollama server after 30 seconds of silence

👂 Ollama Load Balancer listening on http://0.0.0.0:11434

🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64288
⛔😱 Server http://192.168.150.134:11434 didn't respond, now marked Unreliable. Error: error sending request for url (http://192.168.150.134:11434/api/chat)
⚠️  Connection closed with Unreliable Server http://192.168.150.134:11434
☠️  Received CTRL+C, shutting down gracefully...

C:\Users\user\Downloads>
```

### All three servers are off, until 192.168.150.134 is turned back on- but then fails during streaming
```sh
C:\Users\user\Downloads>ollama_load_balancer.exe --version
ollama_load_balancer 1.0.1

C:\Users\user\Downloads>ollama_load_balancer.exe --server http://192.168.150.134:11434 --server http://192.168.150.135:11434 --server http://192.168.150.136:11434

📒 Ollama servers list:
1. http://192.168.150.134:11434
2. http://192.168.150.135:11434
3. http://192.168.150.136:11434

⚙️  Timeout setting: Will abandon Ollama server after 30 seconds of silence

👂 Ollama Load Balancer listening on http://0.0.0.0:11434

🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64308
⛔😱 Server http://192.168.150.134:11434 didn't respond, now marked Unreliable. Error: error sending request for url (http://192.168.150.134:11434/api/chat)
⚠️  Connection closed with Unreliable Server http://192.168.150.134:11434
🤖🦸 Chose reliable server: http://192.168.150.135:11434 to serve client 127.0.0.1:64310
⛔😱 Server http://192.168.150.135:11434 didn't respond, now marked Unreliable. Error: error sending request for url (http://192.168.150.135:11434/api/chat)
⚠️  Connection closed with Unreliable Server http://192.168.150.135:11434
🤖🦸 Chose reliable server: http://192.168.150.136:11434 to serve client 127.0.0.1:64312
⛔😱 Server http://192.168.150.136:11434 didn't respond, now marked Unreliable. Error: error sending request for url (http://192.168.150.136:11434/api/chat)
⚠️  Connection closed with Unreliable Server http://192.168.150.136:11434
🤖😇 Giving server http://192.168.150.134:11434 another chance with client 127.0.0.1:64315
⛔😞 Unreliable server http://192.168.150.134:11434 didn't respond. Error: error sending request for url (http://192.168.150.134:11434/api/chat)
⚠️  Connection closed with Unreliable Server http://192.168.150.134:11434
🤖😇 Giving server http://192.168.150.135:11434 another chance with client 127.0.0.1:64317
⛔😞 Unreliable server http://192.168.150.135:11434 didn't respond. Error: error sending request for url (http://192.168.150.135:11434/api/chat)
⚠️  Connection closed with Unreliable Server http://192.168.150.135:11434
🤖😇 Giving server http://192.168.150.136:11434 another chance with client 127.0.0.1:64319
⛔😞 Unreliable server http://192.168.150.136:11434 didn't respond. Error: error sending request for url (http://192.168.150.136:11434/api/chat)
⚠️  Connection closed with Unreliable Server http://192.168.150.136:11434
🤖😇 Giving server http://192.168.150.134:11434 a 3rd+ chance with client 127.0.0.1:64325
⛔😞 Unreliable server http://192.168.150.134:11434 failed during streaming. Error: error decoding response body
⚠️  Connection closed with Unreliable Server http://192.168.150.134:11434
☠️  Received CTRL+C, shutting down gracefully...

C:\Users\user\Downloads>
```

### A server fails twice, then succeeds twice
```sh
C:\Users\user\Downloads>ollama_load_balancer.exe --version
ollama_load_balancer 1.0.1

C:\Users\user\Downloads>ollama_load_balancer.exe --server http://192.168.150.134:11434

📒 Ollama servers list:
1. http://192.168.150.134:11434

⚙️  Timeout setting: Will abandon Ollama server after 30 seconds of silence

👂 Ollama Load Balancer listening on http://0.0.0.0:11434

🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64355
⛔😱 Server http://192.168.150.134:11434 didn't respond, now marked Unreliable. Error: error sending request for url (http://192.168.150.134:11434/api/chat)
⚠️  Connection closed with Unreliable Server http://192.168.150.134:11434
🤖😇 Giving server http://192.168.150.134:11434 another chance with client 127.0.0.1:64358
⛔😞 Unreliable server http://192.168.150.134:11434 didn't respond. Error: error sending request for url (http://192.168.150.134:11434/api/chat)
⚠️  Connection closed with Unreliable Server http://192.168.150.134:11434
🤖😇 Giving server http://192.168.150.134:11434 a 3rd+ chance with client 127.0.0.1:64365
🙏⚕️  Server http://192.168.150.134:11434 has completed streaming successfully and is now marked Reliable
🟢 Server http://192.168.150.134:11434 now available
🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64378
🟢 Server http://192.168.150.134:11434 now available
☠️  Received CTRL+C, shutting down gracefully...

C:\Users\user\Downloads>
```

### Generation canceled by continue.dev- not an error
```sh
C:\Users\user\Downloads>ollama_load_balancer.exe --version
ollama_load_balancer 1.0.1

C:\Users\user\Downloads>ollama_load_balancer.exe --server http://192.168.150.134:11434

📒 Ollama servers list:
1. http://192.168.150.134:11434

⚙️  Timeout setting: Will abandon Ollama server after 30 seconds of silence

👂 Ollama Load Balancer listening on http://0.0.0.0:11434

🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64417
🟢 Server http://192.168.150.134:11434 now available
☠️  Received CTRL+C, shutting down gracefully...

C:\Users\user\Downloads>
```

### Generation canceled by continue.dev while using unreliable server- streaming did not complete successfully so not enough to mark Reliable, but not to report error either
```sh
C:\Users\user\Downloads>ollama_load_balancer.exe --version
ollama_load_balancer 1.0.1

C:\Users\user\Downloads>ollama_load_balancer.exe --server http://192.168.150.134:11434

📒 Ollama servers list:
1. http://192.168.150.134:11434

⚙️  Timeout setting: Will abandon Ollama server after 30 seconds of silence

👂 Ollama Load Balancer listening on http://0.0.0.0:11434

🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64428
⛔😱 Server http://192.168.150.134:11434 didn't respond, now marked Unreliable. Error: error sending request for url (http://192.168.150.134:11434/api/chat)
⚠️  Connection closed with Unreliable Server http://192.168.150.134:11434
🤖😇 Giving server http://192.168.150.134:11434 another chance with client 127.0.0.1:64431
⚠️  Connection closed with Unreliable Server http://192.168.150.134:11434
☠️  Received CTRL+C, shutting down gracefully...

C:\Users\user\Downloads>
```

### CTRL+C waits for as long as needed until generation is done in all open connections
```sh
C:\Users\user\Downloads>ollama_load_balancer.exe --version
ollama_load_balancer 1.0.1

C:\Users\user\Downloads>ollama_load_balancer.exe --server http://192.168.150.134:11434

📒 Ollama servers list:
1. http://192.168.150.134:11434

⚙️  Timeout setting: Will abandon Ollama server after 30 seconds of silence

👂 Ollama Load Balancer listening on http://0.0.0.0:11434

🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64441
☠️  Received CTRL+C, shutting down gracefully...
🟢 Server http://192.168.150.134:11434 now available

C:\Users\user\Downloads>
```

### First server was off, second server was on. First server marked as unreliable, so for every next generation, the second server is chosen again and again
```sh
C:\Users\user\Downloads>ollama_load_balancer.exe --version
ollama_load_balancer 1.0.1

C:\Users\user\Downloads>ollama_load_balancer.exe --server http://192.168.150.134:11434 --server http://192.168.150.135:11434

📒 Ollama servers list:
1. http://192.168.150.134:11434
2. http://192.168.150.135:11434

⚙️  Timeout setting: Will abandon Ollama server after 30 seconds of silence

👂 Ollama Load Balancer listening on http://0.0.0.0:11434

🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64459
⛔😱 Server http://192.168.150.134:11434 didn't respond, now marked Unreliable. Error: error sending request for url (http://192.168.150.134:11434/api/chat)
⚠️  Connection closed with Unreliable Server http://192.168.150.134:11434
🤖🦸 Chose reliable server: http://192.168.150.135:11434 to serve client 127.0.0.1:64462
🟢 Server http://192.168.150.135:11434 now available
🤖🦸 Chose reliable server: http://192.168.150.135:11434 to serve client 127.0.0.1:64474
🟢 Server http://192.168.150.135:11434 now available
🤖🦸 Chose reliable server: http://192.168.150.135:11434 to serve client 127.0.0.1:64477
🟢 Server http://192.168.150.135:11434 now available
☠️  Received CTRL+C, shutting down gracefully...

C:\Users\user\Downloads>
```

### Nothing ever goes wrong, all requests are single file- so first server is always chosen
```sh
C:\Users\user\Downloads>ollama_load_balancer.exe --version
ollama_load_balancer 1.0.1

C:\Users\user\Downloads>ollama_load_balancer.exe --server http://192.168.150.134:11434 --server http://192.168.150.135:11434

📒 Ollama servers list:
1. http://192.168.150.134:11434
2. http://192.168.150.135:11434

⚙️  Timeout setting: Will abandon Ollama server after 30 seconds of silence

👂 Ollama Load Balancer listening on http://0.0.0.0:11434

🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64510
🟢 Server http://192.168.150.134:11434 now available
🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64513
🟢 Server http://192.168.150.134:11434 now available
🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64516
🟢 Server http://192.168.150.134:11434 now available
🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64518
🟢 Server http://192.168.150.134:11434 now available
☠️  Received CTRL+C, shutting down gracefully...

C:\Users\user\Downloads>
```

### Nonsensical server IP + port doesn't crash utility
```sh
C:\Users\user\Downloads>ollama_load_balancer.exe --version
ollama_load_balancer 1.0.1

C:\Users\user\Downloads>ollama_load_balancer.exe --server aksldjflakje

📒 Ollama servers list:
1. aksldjflakje

⚙️  Timeout setting: Will abandon Ollama server after 30 seconds of silence

👂 Ollama Load Balancer listening on http://0.0.0.0:11434

🤖🦸 Chose reliable server: aksldjflakje to serve client 127.0.0.1:64530
⛔😱 Server aksldjflakje didn't respond, now marked Unreliable. Error: builder error
⚠️  Connection closed with Unreliable Server aksldjflakje
☠️  Received CTRL+C, shutting down gracefully...

C:\Users\user\Downloads>
```

### Set timeout to infinity and pause VM running Ollama server during generation- we will be stuck forever now (or until somebody unpauses the VM)

```sh
C:\Users\user\Downloads>ollama_load_balancer.exe --version
ollama_load_balancer 1.0.1

C:\Users\user\Downloads>ollama_load_balancer.exe --server http://192.168.150.134:11434 --timeout 0

📒 Ollama servers list:
1. http://192.168.150.134:11434

⚙️  Timeout setting: Will abandon Ollama server after 0 seconds of silence

👂 Ollama Load Balancer listening on http://0.0.0.0:11434

🤖🦸 Chose reliable server: http://192.168.150.134:11434 to serve client 127.0.0.1:64565
☠️  Received CTRL+C, shutting down gracefully...

```

## Research

I set up an Ollama server running on my local network.

I then set up Continue.dev to access that Ollama server.

`continue.dev` has a chat like ChatGPT.

I recorded that there is no network traffic between my PC running VS Code and the Ollama server, until I press ENTER in the chat in VS Code- to start streaming a response.

In wireshark I saw the request structure.

First the TCP connection is created: [SYN] to 192.168.150.134:11434, then `[SYN, ACK]` to back to the TCP client at: 192.168.150.1 on a random port (the same port as the origin of the original [SYN]).

Then there's an `[ACK]` back to 192.168.150.134. With that, the TCP connection is established.

The very next thing is an HTTP/1.1 POST request 192.168.150.1 -> 192.168.150.134 at endpoint "/api/chat".

TCP payload:
```txt
POST /api/chat HTTP/1.1
accept: */*
accept-encoding: gzip, deflate, br
authorization: Bearer undefined
content-length: 167
content-type: application/json
user-agent: node-fetch
Host: 192.168.150.134:11434
Connection: close

{"model":"deepseek-coder:1.3b-instruct-q4_0","raw":true,"keep_alive":1800,"options":{"num_predict":2048,"num_ctx":4096},"messages":[{"role":"user","content":"Hello"}]}
```

Essentially, that tells the Ollama server to load the model if needed, and to have the model start working with those settings, and that prompt. In this case "Hello" is indeed the prompt in the chat in the VS Code window.

Then there's a stream of the LLM response, which altogether produces this full text:
```txt
Hi! How can I assist you today? Please provide more details about your question or issue regarding programming languages with the AI assistant if it's related to computer science topics rather than general knowledge issues like hello world programmers etc, so we get a better understanding. (Sorry for any confusion in previous responses) If not specifically asked yet and I am unable to provide an answer as per my current capabilities based on what is provided currently - AI model by Deepseek! Please let me know if there's anything else you need help with over here, whether it be a programming language problem or something completely different.
```

Now let's talk about the resopnse:
It starts with a TCP `[PSH, ACK]` packet 192.168.150.134 -> 192.168.150.1 that contains this 294 bytes TCP payload:
```txt
HTTP/1.1 200 OK
Content-Type: application/x-ndjson
Date: Sat, 19 Oct 2024 19:39:14 GMT
Connection: close
Transfer-Encoding: chunked

95
{"model":"deepseek-coder:1.3b-instruct-q4_0","created_at":"2024-10-19T19:39:14.1898363Z","message":{"role":"assistant","content":"Hi"},"done":false}


```

That TCP packet is the beginning of the response, but there's no HTTP response terminator yet.

Notice that the text I just quoted is the pure payload when copied as printable text. This "HTTP/1.1 200 OK ..." is plain text inside of the TCP payload.

Then there are ~100 packets of that same type `[PSH, ACK]`.
Each `[PSH, ACK]` has an `[ACK]` from 192.168.150.1

Notice the ending double newlines. Each `[PSH, ACK]` ends with a double carriage return. More specifically, these four binary bytes: "\r\n\r\n"

I will paste some of their payloads in order:

Payload: 154 bytes
```txt
94
{"model":"deepseek-coder:1.3b-instruct-q4_0","created_at":"2024-10-19T19:39:14.2585923Z","message":{"role":"assistant","content":"!"},"done":false}


```

Payload: 157 bytes
```txt
97
{"model":"deepseek-coder:1.3b-instruct-q4_0","created_at":"2024-10-19T19:39:14.3346855Z","message":{"role":"assistant","content":" How"},"done":false}


```

Payload: 156 bytes
```txt
97
{"model":"deepseek-coder:1.3b-instruct-q4_0","created_at":"2024-10-19T19:39:14.4049587Z","message":{"role":"assistant","content":" can"},"done":false}


```

Payload: 154 bytes
```txt
94
{"model":"deepseek-coder:1.3b-instruct-q4_0","created_at":"2024-10-19T19:39:14.455463Z","message":{"role":"assistant","content":" I"},"done":false}


```

Then it continues like that for every single word of the response...
and as we approach the end:

Payload: 163 bytes
```txt
9d
{"model":"deepseek-coder:1.3b-instruct-q4_0","created_at":"2024-10-19T19:39:22.9287849Z","message":{"role":"assistant","content":" different"},"done":false}


```

Payload: 154 bytes
```txt
94
{"model":"deepseek-coder:1.3b-instruct-q4_0","created_at":"2024-10-19T19:39:23.0041127Z","message":{"role":"assistant","content":"."},"done":false}


```

Payload: 155 bytes
```txt
95
{"model":"deepseek-coder:1.3b-instruct-q4_0","created_at":"2024-10-19T19:39:23.0705385Z","message":{"role":"assistant","content":"\n"},"done":false}


```

And then, the very last packet before the zero terminator is another `[PSH, ACK]` packet, this time "done" is finally true in the application-specific data format sent here.

The content:
Payload: 326 bytes
```txt
13f
{"model":"deepseek-coder:1.3b-instruct-q4_0","created_at":"2024-10-19T19:39:23.1468105Z","message":{"role":"assistant","content":""},"done_reason":"stop","done":true,"total_duration":9033032700,"load_duration":13675700,"prompt_eval_count":70,"prompt_eval_duration":69277000,"eval_count":127,"eval_duration":8945400000}


```
Notice that done_reason is "stop" meaning, the LLM said enough, and decided to stop.

Then there's a single TCP packet:
Payload: 5 bytes
```txt
0


```
which marks the end of the HTTP response.
Notice that even the zero terminator then ends with "\r\n\r\n", as the HTTP protocol dictates.

Then after the end of the response there are more TCP packets:

1. A TCP `[ACK]` from the VS Code to the packet that marks the end of the HTTP response.

2. `[FIN, ACK]` initiated by the Ollama server

3. `[ACK]` as a response to `[FIN, ACK]`

4. `[FIN, ACK]` initiated again(?) by the VS Code

5. `[ACK]` as a response to `[FIN, ACK]`

With that, the TCP connection is done.

All of this network analysis was the result of of a single ENTER click in that chat window in `continue.dev` as it communicates with Ollama server running on the local network.
