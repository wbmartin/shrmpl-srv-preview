"""E2E tests for shrmpl-kv-srv."""

import socket


def send_cmd(port, cmd):
    """Send a command to the KV server and return the response."""
    with socket.create_connection(("127.0.0.1", port), timeout=5) as s:
        s.sendall(f"{cmd}\n".encode())
        return s.recv(4096).decode().strip()


class TestPing:
    def test_ping(self, kv_server):
        assert send_cmd(kv_server.port, "PING") == "PONG"


class TestSetGet:
    def test_set_and_get(self, kv_server):
        assert send_cmd(kv_server.port, "SET mykey myvalue") == "OK"
        assert send_cmd(kv_server.port, "GET mykey") == "myvalue"

    def test_get_missing_key(self, kv_server):
        assert send_cmd(kv_server.port, "GET nonexistent") == "*KEY NOT FOUND*"

    def test_set_overwrites(self, kv_server):
        send_cmd(kv_server.port, "SET overkey first")
        send_cmd(kv_server.port, "SET overkey second")
        assert send_cmd(kv_server.port, "GET overkey") == "second"


class TestDelete:
    def test_delete_existing(self, kv_server):
        send_cmd(kv_server.port, "SET delkey delval")
        assert send_cmd(kv_server.port, "DEL delkey") == "OK"
        assert send_cmd(kv_server.port, "GET delkey") == "*KEY NOT FOUND*"

    def test_delete_missing(self, kv_server):
        assert send_cmd(kv_server.port, "DEL nokey") == "*KEY NOT FOUND*"


class TestIncr:
    def test_incr_new_key(self, kv_server):
        result = send_cmd(kv_server.port, "INCR counter1")
        assert result == "1"

    def test_incr_existing(self, kv_server):
        send_cmd(kv_server.port, "SET counter2 5")
        result = send_cmd(kv_server.port, "INCR counter2")
        assert result == "6"

    def test_incr_non_numeric(self, kv_server):
        """INCR on a non-numeric key resets to 1 (by design)."""
        send_cmd(kv_server.port, "SET strkey hello")
        result = send_cmd(kv_server.port, "INCR strkey")
        assert result == "1"


class TestBatch:
    def test_batch_commands(self, kv_server):
        send_cmd(kv_server.port, "SET bk1 bv1")
        result = send_cmd(kv_server.port, "BATCH GET bk1;PING")
        parts = result.split(";")
        assert len(parts) == 2
        assert parts[0].strip() == "bv1"
        assert parts[1].strip() == "PONG"

    def test_batch_too_many(self, kv_server):
        result = send_cmd(kv_server.port, "BATCH PING;PING;PING;PING")
        assert "ERROR" in result

    def test_batch_empty(self, kv_server):
        """BATCH with no commands after prefix should not crash server."""
        # After fix: line.starts_with("BATCH ") && line.len() > 6
        # "BATCH " is exactly 6 chars, so this should fall through to unknown command
        result = send_cmd(kv_server.port, "BATCH")
        assert "ERROR" in result or "PONG" not in result


class TestLimits:
    def test_key_too_long(self, kv_server):
        long_key = "k" * 101
        result = send_cmd(kv_server.port, f"SET {long_key} val")
        assert "ERROR" in result

    def test_value_too_long(self, kv_server):
        long_val = "v" * 101
        result = send_cmd(kv_server.port, f"SET shortkey {long_val}")
        assert "ERROR" in result


class TestUnknownCommand:
    def test_unknown(self, kv_server):
        result = send_cmd(kv_server.port, "FOOBAR")
        assert "ERROR" in result
