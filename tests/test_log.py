"""E2E tests for shrmpl-log-srv."""

import glob
import os
import socket
import time


def send_log_message(port, level, host, code, message):
    """Send a properly formatted log message to the log server.

    Protocol: LVL(4) SP HOST(32) SP CODE(12) SP LEN(5) COLON SP MSG LF
    Total header = 58 bytes before message.
    """
    lvl = f"{level:<4}"[:4]
    host_padded = f"{host:<32}"[:32]
    code_padded = f"{code:<12}"[:12]
    msg_bytes = message.encode()
    len_str = f"{len(msg_bytes):05d}"
    line = f"{lvl} {host_padded} {code_padded} {len_str}: {message}\n"
    with socket.create_connection(("127.0.0.1", port), timeout=5) as s:
        s.sendall(line.encode())


class TestLogIngestion:
    def test_activity_log_written(self, log_server):
        """ACTV messages should be written to activity log file."""
        send_log_message(
            log_server.port, "ACTV", "test-host", "TESTCODE", "activity test msg"
        )
        # BufWriter only flushes when next message arrives after 2s elapsed
        time.sleep(2.5)
        send_log_message(
            log_server.port, "ACTV", "test-host", "TESTCODE", "flush trigger"
        )
        time.sleep(0.5)
        files = glob.glob(os.path.join(log_server.data_dir, "activity-*.log"))
        assert len(files) >= 1, "Expected activity log file to be created"
        with open(files[0]) as f:
            content = f.read()
        assert "activity test msg" in content

    def test_error_log_written(self, log_server):
        """ERRO messages should be written to error log file."""
        send_log_message(
            log_server.port, "ERRO", "test-host", "ERRCODE", "error test msg"
        )
        time.sleep(2.5)
        send_log_message(
            log_server.port, "ERRO", "test-host", "ERRCODE", "flush trigger"
        )
        time.sleep(0.5)
        files = glob.glob(os.path.join(log_server.data_dir, "error-*.log"))
        assert len(files) >= 1, "Expected error log file to be created"
        with open(files[0]) as f:
            content = f.read()
        assert "error test msg" in content

    def test_misc_log_written(self, log_server):
        """INFO messages should be written to misc log file."""
        send_log_message(
            log_server.port, "INFO", "test-host", "INFOCODE", "info test msg"
        )
        time.sleep(2.5)
        send_log_message(
            log_server.port, "INFO", "test-host", "INFOCODE", "flush trigger"
        )
        time.sleep(0.5)
        files = glob.glob(os.path.join(log_server.data_dir, "misc-*.log"))
        assert len(files) >= 1, "Expected misc log file to be created"
        with open(files[0]) as f:
            content = f.read()
        assert "info test msg" in content


class TestLogProtocol:
    def test_malformed_message_does_not_crash(self, log_server):
        """Sending garbage should not crash the server."""
        with socket.create_connection(("127.0.0.1", log_server.port), timeout=5) as s:
            s.sendall(b"this is not a valid log message\n")
        # Server should still accept new connections
        time.sleep(0.3)
        send_log_message(
            log_server.port, "INFO", "test-host", "AFTER", "still alive"
        )

    def test_oversized_message_rejected(self, log_server):
        """Messages over 4096 bytes should be rejected without crashing."""
        huge_msg = "X" * 5000
        # This will fail protocol validation but shouldn't crash
        with socket.create_connection(("127.0.0.1", log_server.port), timeout=5) as s:
            lvl = "INFO"
            host_padded = f"{'test-host':<32}"[:32]
            code_padded = f"{'BIGMSG':<12}"[:12]
            len_str = f"{len(huge_msg):05d}"
            line = f"{lvl} {host_padded} {code_padded} {len_str}: {huge_msg}\n"
            s.sendall(line.encode())
        time.sleep(0.3)
        # Server should still be running
        send_log_message(
            log_server.port, "INFO", "test-host", "AFTER2", "still alive after big"
        )


class TestLogFilePermissions:
    def test_log_files_not_world_readable(self, log_server):
        """Log files should be created with 0640 permissions."""
        send_log_message(
            log_server.port, "INFO", "test-host", "PERMTEST", "permission check"
        )
        time.sleep(0.5)
        files = glob.glob(os.path.join(log_server.data_dir, "*.log"))
        assert len(files) >= 1
        for f in files:
            mode = os.stat(f).st_mode & 0o777
            assert mode == 0o640, f"Expected 0640 but got {oct(mode)} for {f}"
