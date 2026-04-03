consider sqllite for shrmpl-log, still dump the text to console, but handling the larger text block might be more natural. could use sqlite to query right there on the server which might be helpful when troubleshooting.



# Shrmpl CICD Webhook Service - Design Summary

## Overview
Web server receives webhook calls (by GUID and URL), routes to appropriate bash scripts, and manages the CI/CD pipeline for UAT deployments.

## Architecture
- Webhook receiver routes by GUID to trigger scripts
- Azure DevOps SSH token stored on isolated server (low-risk, four dev root access)
- Pulls entire UAT branch from DevOps repo
- Build logic lives in DevOps repo as bash scripts
- Web server executes scripts on Linux UAT server

## Logging
- All script output logs to file on UAT server
- Logs viewable on-demand via tail
- Developers not interested in logs; Brandon monitors

## Slack Integration
- Web server fires Slack notification when pipeline starts
- Web server reads log file after script completion and sends results notification
- No Slack integration needed in bash scripts themselves
