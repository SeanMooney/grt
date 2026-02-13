# git-review Gerrit API Usage

**Source project:** git-review
**Source files:** `git_review/cmd.py`
**Status:** Stub
**Informs:** `gerrit-client.md`

## Overview

<!-- TODO: How git-review interacts with Gerrit's APIs -->

## SSH Protocol

<!-- TODO: SSH command interface, connection management -->

### Commands Used
<!-- gerrit review, gerrit query, gerrit ls-projects -->

### Connection Management
<!-- SSH multiplexing, key handling -->

## HTTP/REST Protocol

<!-- TODO: REST API endpoints used, JSON handling -->

### Endpoints Used
### Authentication Methods

## API Version Detection

<!-- TODO: How git-review detects Gerrit version and adapts -->

## Authentication

### HTTP Basic/Digest
### SSH Keys
### .netrc Support
### Cookie-based Auth

## Error Handling

<!-- TODO: API error codes, retry logic, timeout handling -->

## grt Divergences

<!-- TODO: Where grt's API usage will differ:
- reqwest async HTTP vs urllib/requests
- REST-only (no SSH) initially
- Structured error types vs string parsing
-->
