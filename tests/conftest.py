"""
Shared fixtures for shrmpl e2e tests.

Builds binaries once per session, provides helpers to start/stop servers.
"""

import os
import socket
import subprocess
import tempfile
import time

import pytest

PROJECT_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET_DIR = os.path.join(PROJECT_DIR, "target", "debug")


def find_free_port():
    """Find a free TCP port on localhost."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for_port(port, host="127.0.0.1", timeout=10):
    """Wait until a TCP port is accepting connections."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection((host, port), timeout=1):
                return True
        except OSError:
            time.sleep(0.1)
    raise TimeoutError(f"Port {port} not ready after {timeout}s")


def write_env_file(path, env_dict):
    """Write a dict as KEY=VALUE lines to a file."""
    with open(path, "w") as f:
        for k, v in env_dict.items():
            f.write(f"{k}={v}\n")


class ServerProcess:
    """Manages a server subprocess lifecycle."""

    def __init__(self, binary, config_path, port):
        self.binary = binary
        self.config_path = config_path
        self.port = port
        self.process = None

    def start(self):
        self.process = subprocess.Popen(
            [self.binary, self.config_path],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        wait_for_port(self.port)
        return self

    def stop(self):
        if self.process:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
            self.process = None


@pytest.fixture(scope="session", autouse=True)
def build_binaries():
    """Build all binaries once per test session."""
    cargo = os.path.expanduser("~/.cargo/bin/cargo")
    result = subprocess.run(
        [cargo, "build", "--bins"],
        cwd=PROJECT_DIR,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        pytest.fail(f"cargo build failed:\n{result.stderr}")


# --- KV Server ---


@pytest.fixture
def kv_server(tmp_path):
    """Start a shrmpl-kv-srv instance on a random port."""
    port = find_free_port()
    config = tmp_path / "kv.env"
    write_env_file(
        config,
        {
            "BIND_ADDR": f"127.0.0.1:{port}",
            "SLOG_DEST": "",
            "SERVER_NAME": "test-kv",
            "SEND_LOG": "false",
            "LOG_LEVEL": "DEBUG",
            "LOG_CONSOLE": "false",
            "SEND_ACTV": "false",
        },
    )
    srv = ServerProcess(
        os.path.join(TARGET_DIR, "shrmpl-kv-srv"), str(config), port
    )
    srv.start()
    yield srv
    srv.stop()


# --- Log Server ---


@pytest.fixture
def log_server(tmp_path):
    """Start a shrmpl-log-srv instance on a random port."""
    port = find_free_port()
    data_dir = str(tmp_path / "logs")
    os.makedirs(data_dir, exist_ok=True)
    config = tmp_path / "log.env"
    write_env_file(
        config,
        {
            "BIND_ADDR": f"127.0.0.1:{port}",
            "DATA_DIR": data_dir,
        },
    )
    srv = ServerProcess(
        os.path.join(TARGET_DIR, "shrmpl-log-srv"), str(config), port
    )
    srv.data_dir = data_dir
    srv.start()
    yield srv
    srv.stop()


# --- CICD Server ---


@pytest.fixture
def cicd_server(tmp_path):
    """Start a shrmpl-cicd-srv instance on a random port."""
    port = find_free_port()
    hooks_dir = str(tmp_path / "hooks")
    os.makedirs(hooks_dir, exist_ok=True)

    # Create a test hook
    test_script = tmp_path / "test-hook.sh"
    test_script.write_text("#!/bin/bash\necho ok\n")
    os.chmod(str(test_script), 0o755)

    hook_config = os.path.join(hooks_dir, "test-hook-deadbeef.env")
    write_env_file(
        hook_config,
        {
            "HOOK_PROVIDER": "generic",
            "HOOK_SECRET": "test-secret-123",
            "HOOK_SCRIPT": str(test_script),
            "HOOK_TIMEOUT": "10",
            "HOOK_DEDUPE_WINDOW": "5",
        },
    )

    config = tmp_path / "cicd.env"
    write_env_file(
        config,
        {
            "CICD_TLS_MODE": "plain",
            "CICD_LISTEN_ADDR": "127.0.0.1",
            "CICD_LISTEN_PORT": str(port),
            "CICD_HOOKS_DIR": hooks_dir,
            "CICD_MAX_CONCURRENT": "4",
            "CICD_DEFAULT_TIMEOUT": "30",
            "SLOG_DEST": "",
            "SLOG_LEVEL": "DEBUG",
            "SLOG_CONSOLE": "false",
            "SLOG_SEND_ACTV": "false",
            "SLOG_SEND_LOG": "false",
        },
    )
    srv = ServerProcess(
        os.path.join(TARGET_DIR, "shrmpl-cicd-srv"), str(config), port
    )
    srv.start()
    yield srv
    srv.stop()


# --- Nackmon Server ---


@pytest.fixture
def nackmon_server(tmp_path):
    """Start a shrmpl-nackmon-srv instance on a random port."""
    port = find_free_port()
    monitors_dir = str(tmp_path / "monitors")
    os.makedirs(monitors_dir, exist_ok=True)

    # Create a test monitor (every minute, 30 min wait)
    monitor_config = os.path.join(monitors_dir, "test-job-TESTCODE.env")
    write_env_file(
        monitor_config,
        {
            "NACK_CRON": "* * * * *",
            "NACK_DESCRIPTION": "Test monitor for e2e",
            "NACK_WAIT_MIN": "30",
        },
    )

    status_path = "/status/e2e-test-guid-1234"
    config = tmp_path / "nackmon.env"
    write_env_file(
        config,
        {
            "NACK_LISTEN_ADDR": "127.0.0.1",
            "NACK_LISTEN_PORT": str(port),
            "NACK_MONITORS_DIR": monitors_dir,
            "NACK_ESCALATION_MISSES": "3",
            "NACK_ESCALATION_INTERVAL_MIN": "60",
            "NACK_STATUS_PATH": status_path,
            "SLOG_DEST": "",
            "SLOG_LEVEL": "DEBUG",
            "SLOG_CONSOLE": "false",
            "SLOG_SEND_ACTV": "false",
            "SLOG_SEND_LOG": "false",
        },
    )
    srv = ServerProcess(
        os.path.join(TARGET_DIR, "shrmpl-nackmon-srv"), str(config), port
    )
    srv.status_path = status_path
    srv.start()
    yield srv
    srv.stop()
