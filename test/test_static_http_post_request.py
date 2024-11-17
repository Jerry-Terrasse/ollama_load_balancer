# Tested with Python 3.10.6 64-bit on Windows 11 Pro 23H2
# Before running, do: pip install requests

import requests

# URL of the load balancer
base_url = "http://127.0.0.1:11434"

post_url = f'{base_url}/api/chat'

# Headers for the HTTP request
headers = {
    'Accept': '*/*',
    'Content-Type': 'application/json',
}

# JSON payload with "stream": false
json_data = {
    "model": "deepseek-coder:1.3b-instruct-q4_0",
    "raw": True,
    "keep_alive": 1800,
    "options": {
        "num_predict": 2048,
        "num_ctx": 4096
    },
    "stream": False,  # Key change to test non-streaming
    "messages": [
        {
            "role": "user",
            "content": "Hello"
        }
    ]
}

# Send the POST request to the load balancer
response = requests.post(post_url, headers=headers, json=json_data)

# Print the response status code, headers, and body
print("Status Code:", response.status_code)
print("Headers:", response.headers)
print("Response Body:", response.text)

# URL of the Ollama server
get_url = f'{base_url}/api/tags'

# Send the GET request to the Ollama server
response = requests.get(get_url)

# Print the response status code, headers, and body
print("Status Code:", response.status_code)
print("Headers:", response.headers)
print("Response Body:", response.text)
