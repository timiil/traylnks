#!/usr/bin/env python3
"""TrayLnks Linux/WSL terminal bridge. Python 3.8+, standard library only.

shell --state-dir DIR : supervise a shell in the current tab
ssh [--state-dir DIR] -- HOST [SSH options before HOST] : monitored direct SSH
install-hooks --tool codex|claude : merge hooks, retaining a backup
hook --tool codex|claude : fail-open, metadata-only lifecycle callback
"""
import argparse
import base64
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import selectors
import shlex
import signal
import socket
import subprocess
import sys
import tempfile
import termios
import time
import tty
import uuid

VERSION = 1
PREFIX = b'\x1b]777;TrayLnks;'
MAX_FRAME = 16384
MARKERS = ('', '✓', '?', '~', '!')
PHASES = ('idle', 'running', 'completed', 'needs_input', 'error', 'unavailable', 'closed')


def millis():
    return int(time.time() * 1000)


def clean_title(value):
    return ''.join(c for c in str(value) if ord(c) >= 32 and not 127 <= ord(c) <= 159)[:240]


def frame(token, value):
    body = json.dumps(value, ensure_ascii=True, separators=(',', ':')).encode()
    return PREFIX + token.encode('ascii') + b';' + base64.b64encode(body) + b'\x07'


class StreamParser:
    """Incremental VT parser. Private relay frames never reach the terminal."""
    def __init__(self, token, on_event, on_title=None):
        self.token = token.encode('ascii')
        self.on_event = on_event
        self.on_title = on_title
        self.mode = 'normal'
        self.buffer = bytearray()
        self.previous = 0
        self.activity = False

    def feed(self, data):
        out = bytearray()
        self.activity = False
        for byte in data:
            if self.mode == 'normal':
                if byte == 27:
                    self.buffer = bytearray([byte])
                    self.mode = 'escape'
                else:
                    out.append(byte)
                    self.activity |= byte >= 32 and byte != 127
            elif self.mode == 'escape':
                self.buffer.append(byte)
                if byte == 93:
                    self.mode = 'osc'
                elif byte == 91:
                    self.mode = 'csi'
                else:
                    out.extend(self.buffer)
                    self.buffer.clear()
                    self.mode = 'normal'
            elif self.mode == 'csi':
                self.buffer.append(byte)
                if 64 <= byte <= 126 or len(self.buffer) > MAX_FRAME:
                    out.extend(self.buffer)
                    self.buffer.clear()
                    self.mode = 'normal'
            elif self.mode == 'osc':
                self.buffer.append(byte)
                if byte == 7 or self.buffer[-2:] == b'\x1b\\':
                    raw = bytes(self.buffer)
                    end = 1 if byte == 7 else 2
                    body = raw[2:-end]
                    if raw.startswith(PREFIX):
                        parts = body.split(b';', 3)
                        if len(parts) == 4 and parts[2] == self.token:
                            try:
                                value = json.loads(base64.b64decode(parts[3], validate=True))
                                if isinstance(value, dict) and value.get('version') == VERSION:
                                    self.on_event(value)
                            except (ValueError, TypeError):
                                pass
                    elif body.startswith((b'0;', b'2;')) and self.on_title:
                        replacement = self.on_title(body.split(b';', 1)[1].decode('utf-8', 'replace'), raw)
                        out.extend(replacement)
                    else:
                        out.extend(raw)
                    self.buffer.clear()
                    self.mode = 'normal'
                elif len(self.buffer) > MAX_FRAME:
                    private = self.buffer.startswith(PREFIX)
                    if not private:
                        out.extend(self.buffer)
                    self.previous = byte
                    self.buffer.clear()
                    self.mode = 'discard' if private else 'passthrough'
            else:
                if self.mode == 'passthrough':
                    out.append(byte)
                if byte == 7 or (self.previous == 27 and byte == 92):
                    self.mode = 'normal'
                self.previous = byte
        return bytes(out)


class Header:
    def __init__(self, base):
        self.base = clean_title(base)
        self.control = {}

    def render(self, now):
        control = self.control
        marker = control.get('marker', '')
        age = now - control.get('updated_at_ms', 0)
        if not control.get('enabled') or not 0 <= age <= 5000 or marker not in MARKERS:
            marker = ''
        if marker and now < control.get('flash_until_ms', 0) and (now // 500) % 2:
            marker = '·'
        title = (marker + ' ' if marker else '') + clean_title(self.base)
        return ('\x1b]2;' + title + '\x07').encode('utf-8')


class State:
    def __init__(self, session_id):
        self.data = dict(version=VERSION, session_id=session_id,
                         title=Path.cwd().name or 'Terminal', host=socket.gethostname(),
                         tool='', tool_session_id='', run_id='', seq=1, activity_seq=0,
                         heartbeat_ms=millis(), observer_available=True, phase='idle', reason='')
        self.remote_sequences = {}
        self.connection_id = None
        self.remote_last_seen = None
        self.remote_started_at = None
        self.blocked_sessions = set()
        self.primary_process = None

    def change(self, **values):
        if any(self.data.get(key) != value for key, value in values.items()):
            self.data.update(values)
            self.data['seq'] += 1

    def hook(self, tool, event):
        name = event.get('hook_event_name', '')
        if name == 'Notification' and event.get('notification_type') not in (
                'permission_prompt', 'elicitation_dialog', 'agent_needs_input'):
            return
        if name == 'SSHStart':
            self.connection_id = event.get('connection_id')
            self.remote_started_at = time.monotonic()
            self.remote_last_seen = None
            self.remote_sequences.clear()
            self.change(tool='ssh', tool_session_id='', run_id=str(self.connection_id),
                        phase='running', reason='Connecting to SSH host', observer_available=True)
            return
        if name == 'SSHExit':
            if event.get('connection_id') != self.connection_id:
                return
            self.connection_id = None
            self.remote_started_at = None
            self.remote_last_seen = None
            self.change(observer_available=True)
            code = event.get('exit_code', 0)
            if code not in (0, 130, -signal.SIGINT):
                self.change(phase='error', reason='SSH exited with status ' + str(code))
            else:
                self.change(phase='idle', tool='', reason='SSH session ended')
            return
        known = ('SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PostToolUse',
                 'PermissionRequest', 'Notification', 'Stop', 'StopFailure', 'Interrupt', 'SessionEnd')
        if name not in known or tool not in ('codex', 'claude'):
            return
        tool_session = str(event.get('session_id') or self.data['tool_session_id'])[:160]
        process = event.get('process_id')
        process = process if isinstance(process, int) and process > 0 else None
        previous_alive = self.primary_process is None or Path('/proc/' + str(self.primary_process)).exists()
        active = (self.data['tool'], self.data['tool_session_id'])
        incoming = (tool, tool_session)
        if name == 'SessionStart':
            if incoming == active and self.data['run_id']:
                if process: self.primary_process = process
                return  # Compaction/resume must not invent a new turn.
            if incoming != active and self.data['phase'] in ('running', 'needs_input') and self.data['tool'] in ('codex', 'claude') and previous_alive:
                self.blocked_sessions.add(incoming)
                return  # A nested CLI inherits the socket, but cannot replace its parent.
            self.blocked_sessions.discard(incoming)
            self.primary_process = process
        elif incoming in self.blocked_sessions:
            if previous_alive: return
            self.blocked_sessions.discard(incoming)
        elif name != 'UserPromptSubmit' and incoming != active:
            return
        if name == 'SessionStart':
            self.change(tool=tool, tool_session_id=tool_session, run_id='', phase='idle', reason='')
        elif name == 'UserPromptSubmit':
            if process: self.primary_process = process
            self.change(tool=tool, tool_session_id=tool_session, run_id=uuid.uuid4().hex,
                        phase='running', reason='')
        elif name in ('PermissionRequest', 'Notification'):
            self.change(tool=tool, tool_session_id=tool_session, phase='needs_input',
                        reason='Waiting for input or permission')
        elif name in ('PreToolUse', 'PostToolUse'):
            if self.data['run_id']:
                self.change(phase='running', reason='')
        elif name == 'Stop':
            if self.data['run_id']:
                self.change(phase='completed', reason='Top-level reply finished; task success is unconfirmed')
        elif name == 'StopFailure':
            self.change(tool=tool, phase='error', reason=clean_title(event.get('error', 'Agent stopped with an error')))
        elif name == 'Interrupt':
            self.change(phase='idle', reason='Interrupted by user')
        elif name == 'SessionEnd':
            self.change(phase='idle', tool='', reason='Agent session ended')

    def remote(self, value, now=None):
        relay = value.get('relay_id')
        seq = value.get('seq')
        if not self.connection_id or value.get('connection_id') != self.connection_id:
            return
        if not isinstance(relay, str) or not isinstance(seq, int):
            return
        if value.get('phase') not in PHASES or value.get('tool') not in ('', 'codex', 'claude'):
            return
        previous = self.remote_sequences.get(relay, -1)
        if seq < previous:
            return
        self.remote_last_seen = time.monotonic() if now is None else now
        self.change(observer_available=True)
        if seq == previous:
            return  # Heartbeat is fresh even if no lifecycle event changed.
        self.remote_sequences[relay] = seq
        self.change(tool=value['tool'], tool_session_id=clean_title(value.get('tool_session_id', '')),
                    run_id=clean_title(value.get('run_id', '')),
                    phase='idle' if value['phase'] == 'closed' else value['phase'],
                    host=clean_title(value.get('host', 'remote')), reason=clean_title(value.get('reason', '')))

    def expire_remote(self, now=None):
        now = time.monotonic() if now is None else now
        last = self.remote_last_seen if self.remote_last_seen is not None else self.remote_started_at
        if self.connection_id and last is not None and now - last > 30:
            self.change(observer_available=False)


def atomic_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    previous_mode = (path.stat().st_mode & 0o777) if path.exists() else 0o600
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode='w', encoding='utf-8', dir=str(path.parent),
                                         prefix=path.name + '.tmp-', delete=False) as handle:
            temporary = Path(handle.name)
            json.dump(value, handle, ensure_ascii=True)
        os.chmod(str(temporary), previous_mode)
        os.replace(str(temporary), str(path))
    finally:
        if temporary:
            try:
                temporary.unlink()
            except FileNotFoundError:
                pass


def owner_process(tool):
    """Find only the owning CLI identity, never retain its arguments."""
    pid = os.getppid()
    for _ in range(16):
        try:
            raw = Path('/proc/' + str(pid) + '/cmdline').read_bytes().split(b'\0')[:2]
            paths = [os.fsdecode(item) for item in raw if item]
            if any(Path(value).name.startswith(tool) or
                   (tool == 'claude' and ('/claude/versions/' in value or '@anthropic-ai/claude-code/' in value))
                   for value in paths):
                return pid
            fields = Path('/proc/' + str(pid) + '/stat').read_text().rsplit(')', 1)[1].split()
            pid = int(fields[1])
            if pid <= 1: break
        except (OSError, ValueError, IndexError):
            break
    return None


def send_event(tool, event):
    endpoint = os.environ.get('TRAYLNKS_AGENT_SOCKET')
    if not endpoint:
        return
    # Never send prompts, tool arguments, transcripts, or notification bodies.
    event = dict(event)
    if tool in ('codex', 'claude'): event['process_id'] = owner_process(tool)
    event = {key: event[key] for key in ('hook_event_name', 'session_id', 'notification_type', 'error', 'exit_code', 'connection_id', 'source', 'process_id') if key in event}
    message = json.dumps({'tool': tool, 'event': event}, ensure_ascii=True).encode() + b'\n'
    if len(message) > 8192:
        return
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
            client.settimeout(0.3)
            client.connect(endpoint)
            client.sendall(message)
    except OSError:
        pass  # A closed monitor must never block the AI CLI.


def hook_config(tool):
    command = 'python3 ' + shlex.quote(str(Path(__file__).resolve())) + ' hook --tool ' + tool
    events = ['SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PostToolUse', 'PermissionRequest', 'Stop', 'SessionEnd']
    events += ['Interrupt'] if tool == 'codex' else ['Notification', 'StopFailure']
    hooks = {}
    for event in events:
        group = {'hooks': [{'type': 'command', 'command': command, 'timeout': 2}]}
        if event == 'Notification':
            group['matcher'] = 'permission_prompt|elicitation_dialog|agent_needs_input'
        hooks[event] = [group]
    return hooks


def install_hooks(tool, path=None):
    if path is None:
        folder = Path(os.environ.get('CODEX_HOME', str(Path.home() / '.codex'))) if tool == 'codex' else Path(os.environ.get('CLAUDE_CONFIG_DIR', str(Path.home() / '.claude')))
        path = folder / ('hooks.json' if tool == 'codex' else 'settings.json')
    path = Path(path)
    original = path.read_text(encoding='utf-8') if path.exists() else '{}'
    config = json.loads(original)
    if not isinstance(config, dict) or not isinstance(config.get('hooks', {}), dict):
        raise ValueError('Existing hooks configuration is not an object; left unchanged')
    hooks = config.setdefault('hooks', {})
    for event, groups in hook_config(tool).items():
        existing = hooks.setdefault(event, [])
        if not isinstance(existing, list):
            raise ValueError('Existing ' + event + ' hooks are not an array; left unchanged')
        for group in groups:
            command = group['hooks'][0]['command']
            if not any(any(h.get('command') == command for h in g.get('hooks', []) if isinstance(h, dict))
                       for g in existing if isinstance(g, dict)):
                existing.append(group)
    path.parent.mkdir(parents=True, exist_ok=True)
    backup = path.with_suffix(path.suffix + '.traylnks-backup')
    if path.exists() and not backup.exists():
        with backup.open('x', encoding='utf-8') as handle:
            os.chmod(str(backup), 0o600)
            handle.write(original)
    atomic_json(path, config)
    return str(path)


def write_all(fd, data):
    while data:
        try:
            count = os.write(fd, data)
            data = data[count:]
        except InterruptedError:
            continue


def reap_child(child):
    """Bounded cleanup of the PTY session owned by this bridge."""
    try:
        pid, status = os.waitpid(child, os.WNOHANG)
        if pid:
            return status
        for sig in (signal.SIGHUP, signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(child, sig)
            except ProcessLookupError:
                pass
            deadline = time.monotonic() + 0.35
            while time.monotonic() < deadline:
                pid, status = os.waitpid(child, os.WNOHANG)
                if pid:
                    return status
                time.sleep(0.01)
        return os.waitpid(child, 0)[1]
    except ChildProcessError:
        return 0


def supervise(state_dir=None, relay_token=None, command=None, connection_id=None):
    if not os.isatty(0) or not os.isatty(1):
        raise ValueError('Run the monitor inside an interactive terminal tab')
    if not state_dir and not relay_token:
        raise ValueError('Use --state-dir with the monitor folder shown in TrayLnks Settings')
    identity = uuid.uuid4().hex
    token = relay_token or uuid.uuid4().hex
    state = State(identity)
    state.data['relay_id'] = identity
    state.data['connection_id'] = connection_id
    header = Header(state.data['title'])
    directory = Path(state_dir).expanduser() if state_dir else None
    state_path = directory / 'sessions' / (identity + '.json') if directory else None
    control_path = directory / 'control' / (identity + '.json') if directory else None
    runtime = tempfile.TemporaryDirectory(prefix='traylnks-')
    endpoint = str(Path(runtime.name) / 'events.sock')
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(endpoint)
    os.chmod(endpoint, 0o600)
    listener.listen(16)
    listener.setblocking(False)
    original = termios.tcgetattr(0)
    shell = os.environ.get('SHELL', '/bin/sh')
    child, master = pty.fork()
    if child == 0:
        listener.close()
        os.environ['TRAYLNKS_AGENT_SOCKET'] = endpoint
        os.environ['TRAYLNKS_RELAY_TOKEN'] = token
        os.environ['TRAYLNKS_STATE_DIR'] = str(directory) if directory else ''
        argv = command or [shell, '-l']
        try:
            os.execvp(argv[0], argv)
        except OSError as error:
            os.write(2, ('Cannot start shell: ' + str(error) + '\r\n').encode())
            os._exit(127)
    selector = selectors.DefaultSelector()
    selector.register(0, selectors.EVENT_READ, 'stdin')
    selector.register(master, selectors.EVENT_READ, 'pty')
    selector.register(listener, selectors.EVENT_READ, 'listener')
    buffers = {}
    last_header = None
    next_publish = 0
    next_control = 0
    stopped = False
    previous_handlers = {}

    def resize(*_args):
        try:
            size = fcntl.ioctl(0, termios.TIOCGWINSZ, b'\0' * 8)
            fcntl.ioctl(master, termios.TIOCSWINSZ, size)
        except OSError:
            pass

    def stop(*_args):
        nonlocal stopped
        stopped = True

    def title_changed(value, raw):
        header.base = clean_title(value)
        state.data['title'] = header.base
        return raw if relay_token else header.render(millis())

    parser = StreamParser(token, state.remote, title_changed)

    def publish():
        state.data['heartbeat_ms'] = millis()
        if relay_token:
            write_all(1, frame(token, state.data))
        else:
            try:
                atomic_json(state_path, state.data)
            except OSError:
                pass  # Forwarding the terminal remains usable if storage is offline.

    try:
        for sig, handler in ((signal.SIGWINCH, resize), (signal.SIGTERM, stop), (signal.SIGHUP, stop)):
            previous_handlers[sig] = signal.signal(sig, handler)
        resize()
        tty.setraw(0)
        publish()
        while not stopped:
            now = time.monotonic()
            state.expire_remote(now)
            if now >= next_publish:
                publish()
                next_publish = now + 1
            if control_path and now >= next_control:
                try:
                    value = json.loads(control_path.read_text(encoding='utf-8'))
                    if value.get('version') == VERSION and value.get('session_id') == identity:
                        header.control = value
                except (OSError, ValueError, AttributeError):
                    pass
                next_control = now + 0.2
                rendered = header.render(millis())
                if rendered != last_header:
                    write_all(1, rendered)
                    last_header = rendered
            for key, _mask in selector.select(0.1):
                kind = key.data
                if kind == 'stdin':
                    data = os.read(0, 65536)
                    if not data:
                        stopped = True
                        break
                    write_all(master, data)
                elif kind == 'pty':
                    try:
                        data = os.read(master, 65536)
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                        data = b''
                    if not data:
                        stopped = True
                        break
                    output = parser.feed(data)
                    if parser.activity:
                        state.data['activity_seq'] += 1
                    write_all(1, output)
                elif kind == 'listener':
                    connection, _address = listener.accept()
                    connection.setblocking(False)
                    buffers[connection] = bytearray()
                    selector.register(connection, selectors.EVENT_READ, 'client')
                else:
                    connection = key.fileobj
                    data = connection.recv(8192)
                    buffers[connection].extend(data)
                    if b'\n' in buffers[connection] or not data or len(buffers[connection]) > 8192:
                        if len(buffers[connection]) <= 8192:
                            try:
                                message = json.loads(buffers[connection].split(b'\n', 1)[0])
                                state.hook(message.get('tool', ''), message.get('event', {}))
                                publish()  # Hook delivery bypasses the quiet scan interval.
                            except (ValueError, TypeError, AttributeError):
                                pass
                        selector.unregister(connection)
                        del buffers[connection]
                        connection.close()
    finally:
        try:
            termios.tcsetattr(0, termios.TCSADRAIN, original)
        except (OSError, termios.error):
            pass  # Closing the outer tab may already have destroyed its PTY.
        for sig, handler in previous_handlers.items():
            signal.signal(sig, handler)
        selector.close()
        for connection in buffers:
            connection.close()
        listener.close()
        os.close(master)
        status = reap_child(child)
        state.change(phase='closed', reason='Supervised shell closed')
        publish()
        if not relay_token:
            header.control = {}
            write_all(1, header.render(millis()))
        runtime.cleanup()
    code = os.waitstatus_to_exitcode(status) if hasattr(os, 'waitstatus_to_exitcode') else status >> 8
    return code if code >= 0 else 128 - code


def ssh_connect(arguments, state_dir=None):
    if not arguments:
        raise ValueError('Provide SSH options and a destination, e.g. ssh -- user@host')
    endpoint = os.environ.get('TRAYLNKS_AGENT_SOCKET')
    if not endpoint:
        # Keep a supervised local shell alive after SSH exits, so a connection
        # failure remains marked until the user acknowledges or starts again.
        script = str(Path(__file__).resolve())
        invocation = ['python3', script, 'ssh', '--'] + arguments
        shell = os.environ.get('SHELL', '/bin/sh')
        line = ' '.join(shlex.quote(arg) for arg in invocation) + '; exec ' + shlex.quote(shell) + ' -l'
        return supervise(state_dir, command=[shell, '-ic', line])
    token = os.environ['TRAYLNKS_RELAY_TOKEN']
    connection = uuid.uuid4().hex
    send_event('', {'hook_event_name': 'SSHStart', 'connection_id': connection})
    remote = 'exec python3 "$HOME/.local/share/traylnks/traylnks_agent.py" shell --relay-token ' + shlex.quote(token) + ' --connection-id ' + shlex.quote(connection)
    try:
        code = subprocess.call(['ssh', '-t'] + arguments + [remote])
    except OSError as error:
        print('SSH could not start: ' + str(error), file=sys.stderr)
        code = 127
    send_event('', {'hook_event_name': 'SSHExit', 'exit_code': code, 'connection_id': connection})
    return code


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest='action', required=True)
    shell = commands.add_parser('shell', help='Start a supervised shell in this tab')
    shell.add_argument('--state-dir', default=os.environ.get('TRAYLNKS_STATE_DIR'))
    shell.add_argument('--relay-token')
    shell.add_argument('--connection-id')
    shell.add_argument('--command', nargs=argparse.REMAINDER)
    ssh = commands.add_parser('ssh', help='Connect to a host with the remote bridge installed')
    ssh.add_argument('--state-dir', default=os.environ.get('TRAYLNKS_STATE_DIR'))
    ssh.add_argument('ssh_arguments', nargs=argparse.REMAINDER)
    hook = commands.add_parser('hook', help='Lifecycle callback (safe outside a monitored shell)')
    hook.add_argument('--tool', choices=['codex', 'claude'], required=True)
    install = commands.add_parser('install-hooks', help='Merge lifecycle hooks; backup existing configuration')
    install.add_argument('--tool', choices=['codex', 'claude'], required=True)
    args = parser.parse_args()
    if args.action == 'hook':
        try:
            payload = json.loads(sys.stdin.buffer.read(1024 * 1024))
            if isinstance(payload, dict):
                send_event(args.tool, payload)
        except (ValueError, OSError):
            pass
        print('{}')
        return 0
    if args.action == 'install-hooks':
        print('Hooks merged into ' + install_hooks(args.tool))
        print('Review/trust the hooks in the tool, then restart the AI CLI inside a monitored shell.')
        return 0
    if args.action == 'ssh':
        arguments = args.ssh_arguments
        if arguments and arguments[0] == '--':
            arguments = arguments[1:]
        return ssh_connect(arguments, args.state_dir)
    return supervise(args.state_dir, args.relay_token, args.command, args.connection_id)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, OSError) as error:
        print('TrayLnks: ' + str(error), file=sys.stderr)
        sys.exit(1)
