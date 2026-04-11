"""E2E tests for shrmpl-pulsecheck-srv."""

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


class TestHealth:
    def test_health_endpoint(self, pulsecheck_server):
        status, body = http_get(pulsecheck_server.port, "/health")
        assert status == 200
        data = json.loads(body)
        assert data["status"] == "ok"
        assert data["endpoints_loaded"] == 1
        assert "uptime_seconds" in data

    def test_health_returns_json(self, pulsecheck_server):
        status, body = http_get(pulsecheck_server.port, "/health")
        data = json.loads(body)
        assert isinstance(data["uptime_seconds"], int)


class TestStatusEndpoint:
    def test_status_returns_endpoints(self, pulsecheck_server):
        status, body = http_get(
            pulsecheck_server.port, pulsecheck_server.status_path
        )
        assert status == 200
        data = json.loads(body)
        assert isinstance(data, list)
        assert len(data) == 1
        assert data[0]["code"] == "SELFTST"
        assert data[0]["name"] == "self-check"

    def test_status_wrong_path(self, pulsecheck_server):
        """Status at wrong GUID path should return 404."""
        status, body = http_get(pulsecheck_server.port, "/status/wrong-guid")
        assert status == 404

    def test_status_bare_path(self, pulsecheck_server):
        """GET /status without GUID should return 404."""
        status, body = http_get(pulsecheck_server.port, "/status")
        assert status == 404

    def test_status_fields(self, pulsecheck_server):
        """Status entries should have all expected fields."""
        status, body = http_get(
            pulsecheck_server.port, pulsecheck_server.status_path
        )
        data = json.loads(body)
        entry = data[0]
        expected_fields = [
            "code", "name", "url", "is_healthy", "last_check",
            "last_status", "alert_count", "cert_expiry",
        ]
        for field in expected_fields:
            assert field in entry, f"Missing field: {field}"


class TestStartupCheck:
    def test_endpoint_checked_after_startup(self, pulsecheck_server):
        """After startup, the self-referencing endpoint should be checked."""
        # Wait for the first check cycle
        time.sleep(3)
        status, body = http_get(
            pulsecheck_server.port, pulsecheck_server.status_path
        )
        assert status == 200
        data = json.loads(body)
        endpoints = {e["code"]: e for e in data}
        ep = endpoints["SELFTST"]
        assert ep["last_check"] is not None
        assert ep["is_healthy"] is True
        assert ep["last_status"] == 200
        assert ep["alert_count"] == 0


class TestUnhealthyEndpoint:
    def test_dead_endpoint_detected(self, pulsecheck_server_with_dead_endpoint):
        """An endpoint pointing at a dead port should be marked unhealthy."""
        # Wait for the check to run + timeout (10s) + buffer
        time.sleep(15)
        status, body = http_get(
            pulsecheck_server_with_dead_endpoint.port,
            pulsecheck_server_with_dead_endpoint.status_path,
        )
        assert status == 200
        data = json.loads(body)
        endpoints = {e["code"]: e for e in data}
        ep = endpoints["DEADTST"]
        assert ep["is_healthy"] is False
        assert ep["alert_count"] >= 1
        assert ep["last_check"] is not None


class TestUnknownRoutes:
    def test_unknown_path(self, pulsecheck_server):
        status, body = http_get(pulsecheck_server.port, "/nonexistent")
        assert status == 404

    def test_checkin_not_available(self, pulsecheck_server):
        """Pulsecheck does not have a /checkin endpoint."""
        status, body = http_get(pulsecheck_server.port, "/checkin")
        assert status == 404
