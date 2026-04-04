consider sqllite for shrmpl-log, still dump the text to console, but handling the larger text block might be more natural. could use sqlite to query right there on the server which might be helpful when troubleshooting.


# Security backlog

- shrmpl-vault-cli: Remove `DangerousNoVerification` and implement proper TLS certificate validation. Currently disables all cert checks, enabling MITM attacks if used outside localhost dev.

- shrmpl-cicd-srv: Add per-IP and per-GUID rate limiting on the webhook endpoint. Currently only has max_concurrent protection, which doesn't prevent a single source from spamming requests.
