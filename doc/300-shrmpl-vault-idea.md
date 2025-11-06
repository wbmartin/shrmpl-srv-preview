I want to continue with the shrmpl concept and build a vault.  the intend is to replace a typical cloud key-vault concept that would hold secrets, could be accessed securely over the internet, but would also be able to hold configuration files.  (logically there is no difference to me, they need to be secure too).  I want the rust server to offer an https service with mutual tls.  the client will need to present a client certificate to ask the server for the config.  the config files will be kept in a single directory and be simple text files (contents could be a paragraph, json, yaml, doesn't matter).  the files will be named following the convention "[environment]-[appname]-[friendlyname]-[guid]", e.g. "dev_simple-example_app-server-config-json_08ff3053-b7ba-4f8a-a0d5-b4107c3fc319"

When starting the server, I want to pass one argument - the configuration file that should have these variables, suggest more if you need them
```
BIND_ADDR=0.0.0.0:7474
SLOG_DEST=127.0.0.1:7379
SERVER_NAME=shrmpl-vault-loc
SEND_LOG=true
LOG_LEVEL=DEBUG
LOG_CONSOLE=true
SEND_ACTV=false
TLS_CERTIFICATE_PRIVKEY_PATH=/Users/brandon/Code/simple-server/etc/privkey.pem
TLS_CERTIFICATE_FULLCHAIN_PATH=/Users/brandon/Code/simple-server/etc/fullchain.pem
```

I'd like it to use rusttls and focus on simplicity, following the same approach as teh other shrmpl comments.

I think we can use opentls to generate the certificates.  If so, write the procedure to generate any tls and mutual tls certificates in the technical specs.


You will need to create a simple test client that will take one argument - it will also accept a single argument for the configuration file.  You should also generate a configuration file in the etc folder for the client that included the secret name to retrieve and the the path to the mtls cert it will need to present

at the end of the day, I want a client that can present a mtls certificate and request a filename that the web server will deliver contents as plain text.  If the file doesn't exit it should respond with a 404 and other expected error messages.


Security & Authentication
1. Client Certificate Management: How will you manage client certificate lifecycle? Should there be multiple client certificates, or just one? How do you revoke compromised certificates?
We won't revoke certificates.  Instead we should add a list of allowed secrets.  The client will have to present a secret key in the query string that matches one from the list we'll add in the config file.  That way we can revoke the secret from the config file and block unauthorized use that way.

2. Access Control: Should all clients with valid certificates access all files, or do you need per-client access controls? Should the filename pattern include client identity?
Yes, valid certificate gets access to all files, the guid will prevent them from accessing files they don't have access to. we'll handle per client access by filename convention.  I updated the example.

3. Certificate Validation: Beyond basic mTLS validation, should you implement certificate pinning, expiration checks, or additional security layers?
we don't need certificate pinning. client and server should check for expiration of certificates.  We need someway to see when certificate are expiring.

File Management
4. File Operations: Besides retrieval, do you need file upload, deletion, or listing capabilities? Or just GET operations?
For right now, just GET operations.  we'll administer the configuration files from the OS for now

5. File Organization: The naming convention [environment]-[friendlyname]-[guid] is good, but should the server validate this format? Should there be subdirectories?
No subdirectores or server validation of format for now.

6. Hot Reloading: Should the server detect file changes without restart? What about concurrent access?
The server will need to be restarted to load the configuration file.  The server should always serve the current file on disk, not cached versions.

Configuration & Deployment
7. Config File Format: You mentioned environment variables - should this be a .env file, TOML, JSON, or another format?
I'd like to follow the .env format, but need a nice way to handle that list of secrets.

8. Multiple Environments: How will you handle dev/staging/prod deployments? Should the server be environment-aware?
Server won't know.

9. Service Discovery: How will clients discover the vault server? DNS, service registry, or hardcoded addresses?
They'll be provided a hardcoded address

Error Handling & Monitoring
10. Error Responses: Beyond 404, what specific error codes do you need? Authentication failures, certificate errors, rate limiting?
Yest to all.

11. Logging Integration: You mentioned SLOG integration - what specific events should be logged? Access attempts, failures, file retrievals?
yeah, we should log the requesting IP address for all request what they requested and if it was successful.  It should also have log message levels like debug


12. Health Checks: Should the server expose health check endpoints? What metrics should be available?
yes, but we can just create a file called "healthcheck" in the directory with contents "ok"

Performance & Scalability
13. Caching: Should frequently accessed files be cached in memory? What's the expected file size range?
Files will all be less than 3K, most on the order of a couple hundred bytes

14. Rate Limiting: Should you implement rate limiting per client certificate to prevent abuse?
Yes, that is a great idea.

15. Concurrent Connections: What's the expected concurrent client load?
it should handle 3-5 clients, but they'll only get their config when they need it, most on startup so load should not be material
