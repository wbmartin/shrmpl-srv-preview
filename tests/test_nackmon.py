"""E2E tests for shrmpl-nackmon-srv."""

import json
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
    def test_health_endpoint(self, nackmon_server):
        status, body = http_get(nackmon_server.port, "/health")
        assert status == 200
        data = json.loads(body)
        assert data["status"] == "ok"
        assert data["monitors_loaded"] == 1
        assert "uptime_seconds" in data


class TestCheckin:
    def test_valid_checkin(self, nackmon_server):
        """Check-in with a known code should return 200."""
        status, body = http_get(nackmon_server.port, "/checkin?code=TESTCODE")
        assert status == 200
        data = json.loads(body)
        assert data["status"] == "ok"

    def test_unknown_code(self, nackmon_server):
        """Check-in with unknown code should return 404."""
        status, body = http_get(nackmon_server.port, "/checkin?code=NOPE1234")
        assert status == 404

    def test_missing_code_param(self, nackmon_server):
        """Check-in without code param should return 400."""
        status, body = http_get(nackmon_server.port, "/checkin")
        assert status == 400

    def test_code_too_short(self, nackmon_server):
        """Code under 4 chars should return 400."""
        status, body = http_get(nackmon_server.port, "/checkin?code=AB")
        assert status == 400

    def test_code_too_long(self, nackmon_server):
        """Code over 25 chars should return 400."""
        long_code = "A" * 26
        status, body = http_get(nackmon_server.port, f"/checkin?code={long_code}")
        assert status == 400

    def test_checkin_resets_state(self, nackmon_server):
        """After check-in, status should show 0 consecutive misses."""
        http_get(nackmon_server.port, "/checkin?code=TESTCODE")
        status, body = http_get(
            nackmon_server.port, nackmon_server.status_path
        )
        assert status == 200
        data = json.loads(body)
        monitors = {m["code"]: m for m in data}
        assert monitors["TESTCODE"]["consecutive_misses"] == 0
        assert monitors["TESTCODE"]["last_checkin"] is not None


class TestStatusEndpoint:
    def test_status_returns_monitors(self, nackmon_server):
        status, body = http_get(
            nackmon_server.port, nackmon_server.status_path
        )
        assert status == 200
        data = json.loads(body)
        assert isinstance(data, list)
        assert len(data) == 1
        assert data[0]["code"] == "TESTCODE"
        assert data[0]["name"] == "test-job"
        assert data[0]["description"] == "Test monitor for e2e"

    def test_status_wrong_path(self, nackmon_server):
        """Status at wrong GUID path should return 404."""
        status, body = http_get(nackmon_server.port, "/status/wrong-guid")
        assert status == 404

    def test_status_bare_path(self, nackmon_server):
        """GET /status without GUID should return 404."""
        status, body = http_get(nackmon_server.port, "/status")
        assert status == 404


class TestUnknownRoutes:
    def test_unknown_path(self, nackmon_server):
        status, body = http_get(nackmon_server.port, "/nonexistent")
        assert status == 404


class TestQueryStringLimits:
    def test_huge_query_string(self, nackmon_server):
        """Very long query string should not crash the server."""
        huge_qs = "x=y&" * 5000
        status, body = http_get(
            nackmon_server.port, f"/checkin?{huge_qs}code=TESTCODE"
        )
        # May return 400 (code not found due to truncation) or 200 - either is fine
        # The important thing is the server didn't crash
        assert status in (200, 400)
        # Verify server is still responsive
        status2, _ = http_get(nackmon_server.port, "/health")
        assert status2 == 200
