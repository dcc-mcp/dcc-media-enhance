"""Private Core skill bridge. Reuses Core envelopes and existing CLI lifecycle."""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from dcc_mcp_core import CancelledError, check_cancelled, skill_error, skill_success

NO_PATH = {'capabilities', 'doctor', 'schema'}

def invoke(command: str, path: str | None = None, timeout_secs: int = 900) -> dict:
    if command not in NO_PATH | {'probe', 'plan', 'run', 'batch', 'status', 'cancel'}:
        return skill_error('Unsupported media command.', 'USAGE')
    if type(timeout_secs) is not int or not 1 <= timeout_secs <= 86400:
        return skill_error('timeout_secs must be 1..86400.', 'INVALID_LIMITS')
    binary = Path(os.environ.get('DCC_MEDIA_ENHANCE_BIN', ''))
    digest = os.environ.get('DCC_MEDIA_ENHANCE_SHA256', '')
    if not binary.is_absolute() or not binary.is_file():
        return skill_error('Configure DCC_MEDIA_ENHANCE_BIN to an explicit installed executable.', 'BACKEND_MISSING')
    with binary.open('rb') as source:
        actual = hashlib.file_digest(source, 'sha256').hexdigest()
    if len(digest) != 64 or actual != digest.lower():
        return skill_error('DCC_MEDIA_ENHANCE_SHA256 must match the selected executable.', 'BINARY_IDENTITY')
    argv = [str(binary), command]
    if command not in NO_PATH:
        if not isinstance(path, str) or not Path(path).is_absolute():
            return skill_error('Pass an absolute media, saved job or state path.', 'INVALID_PATH')
        argv.append(path)
    elif path is not None:
        return skill_error('This command does not accept a path.', 'USAGE')
    # Core owns async task IDs. The CLI owns saved media job states. Neither is
    # replaced by an adapter-specific protocol or hidden detached queue.
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        child = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr)
        deadline = time.monotonic() + timeout_secs
        failure = None
        try:
            while child.poll() is None:
                # Meaningful only when Core executes this bridge in process and
                # supplies a token. Default subprocess execution has no token;
                # callers must use the explicit media cancel tool there.
                check_cancelled()
                if time.monotonic() >= deadline:
                    failure = ('Skill invocation exceeded its explicit outer budget.', 'TIMEOUT')
                    break
                if stdout.tell() > 16 * 1024 * 1024 or stderr.tell() > 4 * 1024 * 1024:
                    failure = ('CLI response exceeded the adapter output limit.', 'OUTPUT_LIMIT')
                    break
                time.sleep(.05)
        except CancelledError:
            failure = ('Core requested cancellation; inspect the media job state.', 'CANCELLED')
        finally:
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=5)
        if failure:
            return skill_error(*failure, command=command, exit_code=child.returncode,
                prompt='Inspect the CLI-owned state and retained source before retrying.')
        stdout.seek(0)
        raw = stdout.read(16 * 1024 * 1024 + 1)
        if len(raw) > 16 * 1024 * 1024:
            return skill_error('CLI response exceeded 16 MiB.', 'OUTPUT_LIMIT')
        try:
            payload = json.loads(raw)
            if not isinstance(payload, dict):
                raise ValueError('Expected an object')
        except (ValueError, UnicodeError):
            return skill_error('CLI did not return its JSON contract.', 'INVALID_RESPONSE', exit_code=child.returncode)
        if child.returncode or payload.get('success') is False:
            error = payload.get('error') or {}
            return skill_error(error.get('message', 'Media command failed; inspect per-job results.'),
                error.get('code', 'BATCH_FAILED'), command=command, exit_code=child.returncode, media=payload)
        return skill_success('Media command completed.', command=command, exit_code=child.returncode, media=payload)

def invoke_checked(allowed: set[str], params: dict) -> dict:
    try:
        if not isinstance(params, dict) or set(params) - {'command', 'path', 'timeout_secs'}:
            raise ValueError('Unexpected request fields')
        if params.get('command') not in allowed:
            raise ValueError('Command does not belong to this tool')
        return invoke(**params)
    except (OSError, ValueError, TypeError) as exc:
        return skill_error(str(exc), 'INVALID_REQUEST')


def cli(allowed: set[str]) -> None:
    try:
        raw = sys.stdin.read(65537)
        if len(raw) > 65536:
            raise ValueError('Skill request exceeds 64 KiB')
        result = invoke_checked(allowed, json.loads(raw))
    except (OSError, ValueError, TypeError) as exc:
        result = skill_error(str(exc), 'INVALID_REQUEST')
    print(json.dumps(result))
