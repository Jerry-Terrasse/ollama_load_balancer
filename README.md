# Ollama Load Balancer
Rust utility that load balances multiple https://ollama.com/ servers

## Purpose
Hardware for an Ollama server is expensive. This load balancer allows to distribute a limited number of Ollama servers optimally to multiple developers on a local network.

Let's say you have 60 developers using this service and 6 Ollama servers. What's the probability that 10% of more of your workforce is prompting the LLM at the same time?

## Principal
All developers on the network configure their `continue.dev` (VS Code extension) to point to the IP address of this load balancer instead of manually choosing a specific Ollama server.

Any HTTP POST request for an LLM completion from a developer triggers our app to make an HTTP POST request on behalf of the developer, while streaming the response back to the developer.

We only choose servers that are currently available, we can know which Ollama servers are available based on the assumption that developers only access the Ollama servers via this load balancer.

## Supported Usages
We support not only `continue.dev` but also any client that streams responses from an Ollama server such as https://openwebui.com/

We support both `/api/chat` and `/api/generate` (CTRL+i in `continue.dev`), and actually we support any POST request that is based on streaming with `Transfer-Encoding: chunked` and `Content-Type: application/x-ndjson`.

## Streaming

The LLM doesn't have the complete response immediately which is why Ollama streams the completions.

Streaming is implemented using `Newline Delimited JSON format` (ndjson). See `Content-Type: application/x-ndjson`.

Each line of the ndjson format is mapped to one object in a JSON array.

## Research

I set up an Ollama server running on my local network.

I then configured `continue.dev` (VS Code extension) to access the Ollama server at: http://192.168.150.134:11434/

The `continue.dev` VS Code extension config.json:
```json
{
  "models": [
    {
      "title": "DeepSeek Coder",
      "provider": "ollama",
      "apiBase": "http://192.168.150.134:11434/",
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

`continue.dev` has a chat like ChatGPT.

I recorded that there is no network traffic between my PC running VS Code and the Ollama server, until I press ENTER in the chat in VS Code- to start streaming a response.

In wireshark I see the request structure.

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
