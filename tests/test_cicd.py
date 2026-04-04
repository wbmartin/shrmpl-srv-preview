"""E2E tests for shrmpl-cicd-srv."""

import json
import time
import urllib.request
import urllib.error


def http_get(port, path):
    """Make a GET request and return (status_code, body)."""
    url = f"http://127.0.0.1:{port}{path}"
    try:
        resp = urllib.request.urlopen(url, timeout=5)
        return resp.status, resp.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def http_post(port, path, body="", headers=None):
    """Make a POST request and return (status_code, body)."""
    url = f"http://127.0.0.1:{port}{path}"
    req = urllib.request.Request(url, data=body.encode(), method="POST")
    if headers:
        for k, v in headers.items():
            req.add_header(k, v)
    try:
        resp = urllib.request.urlopen(req, timeout=5)
        return resp.status, resp.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


class TestHealth:
    def test_health_endpoint(self, cicd_server):
        status, body = http_get(cicd_server.port, "/health")
        assert status == 200
        data = json.loads(body)
        assert data["status"] == "ok"
        assert data["hooks_loaded"] == 1
        assert "uptime_seconds" in data


class TestHookEndpoint:
    def test_valid_hook_trigger(self, cicd_server):
        """POST to /hook/<guid> with valid secret should be accepted."""
        status, body = http_post(
            cicd_server.port,
            "/hook/deadbeef",
            body='{"ref": "refs/heads/main"}',
            headers={
                "X-Hook-Secret": "test-secret-123",
                "Content-Type": "application/json",
            },
        )
        assert status == 200
        # Give script time to run
        time.sleep(1)

    def test_invalid_secret(self, cicd_server):
        """POST with wrong secret should be rejected (200 with rejected status)."""
        # CICD returns 200 with status:rejected so webhook providers don't retry
        status, body = http_post(
            cicd_server.port,
            "/hook/deadbeef",
            body="{}",
            headers={
                "X-Hook-Secret": "wrong-secret",
                "Content-Type": "application/json",
            },
        )
        assert status == 200
        data = json.loads(body)
        assert data["status"] == "rejected"

    def test_missing_secret_header(self, cicd_server):
        """POST without X-Hook-Secret should be rejected (200 with rejected status)."""
        status, body = http_post(
            cicd_server.port,
            "/hook/deadbeef",
            body="{}",
            headers={"Content-Type": "application/json"},
        )
        assert status == 200
        data = json.loads(body)
        assert data["status"] == "rejected"

    def test_unknown_guid(self, cicd_server):
        """POST to unknown GUID should return 404."""
        status, body = http_post(
            cicd_server.port,
            "/hook/nonexistent",
            body="{}",
            headers={
                "X-Hook-Secret": "test-secret-123",
                "Content-Type": "application/json",
            },
        )
        assert status == 404


class TestStatusEndpoint:
    def test_status_known_guid(self, cicd_server):
        status, body = http_get(cicd_server.port, "/status/deadbeef")
        assert status == 200
        data = json.loads(body)
        assert data["guid"] == "deadbeef"
        assert data["state"] in ("idle", "running")

    def test_status_unknown_guid(self, cicd_server):
        status, body = http_get(cicd_server.port, "/status/unknown")
        assert status == 404

    def test_status_empty_guid(self, cicd_server):
        """GET /status/ with no GUID should return 404, not panic."""
        status, body = http_get(cicd_server.port, "/status/")
        assert status == 404


class TestHookEmptyGuid:
    def test_hook_empty_guid(self, cicd_server):
        """POST /hook/ with no GUID should return 404, not panic."""
        status, body = http_post(
            cicd_server.port,
            "/hook/",
            body="{}",
            headers={
                "X-Hook-Secret": "test-secret-123",
                "Content-Type": "application/json",
            },
        )
        assert status == 404


class TestUnknownRoutes:
    def test_unknown_path(self, cicd_server):
        status, body = http_get(cicd_server.port, "/nonexistent")
        assert status == 404

    def test_get_on_hook(self, cicd_server):
        """GET on /hook/ path should return 404 (only POST allowed)."""
        status, body = http_get(cicd_server.port, "/hook/deadbeef")
        assert status == 404
