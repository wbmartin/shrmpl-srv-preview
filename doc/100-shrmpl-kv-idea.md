# Shrmpl-KV Summary

This project will replace a redis server and be backwards compatible with redis commands. It will offer a limited number of functions, target smaller implementations (e.g. 3 clients). This is a collection of rough notes and thoughts to jump start the technical spec.


# Desired Outcome
Create two executables - shrmpl-kv-srv and shrmpl-kv-cli (client/server pair) using rust.

Both Executables will take two command line arguments on start - the IP and port to run/connect to. e.g. if I run "shrmpl-kv-srv 127.0.0.1 4141" it will only serve on localhost but if i run "shrmpl-kv-srv 0.0.0.0 4141" it will answer to other hosts on the network.

The commands the server needs to enable are:
* GET recieves 1 argument keyname returns a value.
* SET recieves 2 arguments keyname and value.  it returns an "ok" if successful or an error message if not
* INCR receives 1 arguments keyname.  it returns an value increased by 1.Missing Keys are treated as 0. if client tries to increment a string it returns 0
* PING receives 0 arguments. repsonds with PONG
* DEL receives 1 arguement key name and removes the key/value from the hash

## Shrmpl-KV Architecture & Implementation Notes
- minimize dependencies
- dashmap may be useful crate, but stick to simple hashmaps
- No reconnect per request. Keep 1 connection per client open for hours/days. Send heartbeats (PING/PONG) every ~20–30s to survive NAT/LB idles. if it is possible for the server to send a UPONG (unsolicited PONG) every 2 minutes to keep the connection alive, the client should ignore it.
- Pipelining = speed. Let the client write multiple commands back-to-back without waiting for replies; the server reads/executes in order and streams responses.
- TCP_NODELAY. Disable Nagle on both ends for snappy small writes.
- Simple framing. Use a newline protocol
- For each accepted socket: set_nodelay(true), set_keepalive(Some(60s)).
- we only need to handle integers and string values for now
- We only need to handle 3-5 clients
- key names should be rejected if they have more than 50 characters
- values should be rejected if they have more than 50 characters
- if a value can be converted to an int, it should be treated as an int


## Shrmpl-KV Open questions
- should we use RWLock or Mutex for locking
- should the hashmaps use an enum or have different hashmaps
- What is the exact wire protocol format for commands and responses? For example, are commands like "GET key\n" and responses like "value\n"?
Yes, goal is to mirror redis.
- For INCR on a string value, should it return an error instead of 0?
No, it should start with a 0 and return 1 the first time it is called
- Should values be stored as an enum (Int/String) or always as strings with on-demand parsing?
I am open to either. if we have to choose, opt for readability over speed.
- Should the server use async I/O (e.g., tokio) for concurrency, or blocking threads?
I think async concurrency, bottom line is it should only lock for reading if it is a get and and only block for writing if it is a write.  If someone get a stale value while a transaction is in flight, that is fine for this application.
- How should the server handle and send heartbeat PONGs to clients?
I'm imagining it will have a thread that sends a pong every 2 minutes.  maybe it can send UPONG to say it is an unsolicited PONG so the client can discard it.
