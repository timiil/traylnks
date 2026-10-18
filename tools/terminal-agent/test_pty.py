"""Real process/PTY integration tests; no user's shell or CLI config is changed."""
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import unittest

AGENT = str(Path(__file__).with_name('traylnks_agent.py').resolve())


def wait_until(predicate, timeout=6):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.025)
    raise AssertionError('Timed out waiting for integration condition')


class PtyTests(unittest.TestCase):
    def spawn(self, root, code):
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 90, 0, 0))
        original = termios.tcgetattr(slave)
        process = subprocess.Popen([sys.executable, AGENT, 'shell', '--state-dir', root,
                                    '--command', sys.executable, '-c', code], stdin=slave,
                                   stdout=slave, stderr=slave, start_new_session=True)
        self.addCleanup(lambda: process.poll() is None and process.kill())
        self.addCleanup(os.close, master)
        self.addCleanup(os.close, slave)
        return process, master, slave, original

    def read_available(self, master):
        result = bytearray()
        while select.select([master], [], [], 0)[0]:
            try:
                data = os.read(master, 65536)
            except OSError:
                break
            if not data:
                break
            result.extend(data)
        return bytes(result)

    def test_hook_delivery_header_and_terminal_restoration(self):
        with tempfile.TemporaryDirectory() as root:
            code = '''import json, subprocess, sys, time, os
agent = sys.argv[0]
def hook(name):
 subprocess.run([sys.executable, %r, 'hook', '--tool', 'codex'], input=json.dumps({'hook_event_name':name,'session_id':'s','prompt':'PRIVATE_PROMPT'}), text=True, stdout=subprocess.DEVNULL)
hook('SessionStart'); hook('UserPromptSubmit')
print('中文_OUTPUT', flush=True); time.sleep(.5); hook('Stop'); time.sleep(2)
''' % AGENT
            process, master, slave, original = self.spawn(root, code)
            def completed():
                for path in (Path(root) / 'sessions').glob('*.json'):
                    state = json.loads(path.read_text())
                    if state['phase'] == 'completed':
                        return path, state
                return None
            path, state = wait_until(completed)
            control = Path(root) / 'control' / path.name
            control.parent.mkdir(exist_ok=True)
            control.write_text(json.dumps({'version': 1, 'session_id': state['session_id'],
                'enabled': True, 'marker': '✓', 'updated_at_ms': int(time.time()*1000), 'flash_until_ms': 0}))
            output = bytearray()
            def header():
                output.extend(self.read_available(master))
                return '✓'.encode() in output
            wait_until(header)
            self.assertEqual(process.wait(timeout=5), 0)
            output.extend(self.read_available(master))
            self.assertIn('中文_OUTPUT'.encode(), output)
            self.assertNotIn(b'PRIVATE_PROMPT', path.read_bytes())
            self.assertEqual(termios.tcgetattr(slave), original)
            self.assertEqual(json.loads(path.read_text())['phase'], 'closed')

    def test_ctrl_c_is_forwarded_to_child(self):
        with tempfile.TemporaryDirectory() as root:
            code = "import signal,time; signal.signal(signal.SIGINT,lambda *a:exit(0)); print('READY',flush=True); time.sleep(30)"
            process, master, slave, original = self.spawn(root, code)
            output = bytearray()
            def ready():
                output.extend(self.read_available(master))
                return b'READY' in output
            wait_until(ready)
            os.write(master, b'\x03')
            self.assertEqual(process.wait(timeout=5), 0)
            self.assertEqual(termios.tcgetattr(slave), original)

    def test_terminating_bridge_does_not_hang_on_a_child_ignoring_hangup(self):
        with tempfile.TemporaryDirectory() as root:
            code = "import os,signal,time; signal.signal(signal.SIGHUP,signal.SIG_IGN); signal.signal(signal.SIGTERM,signal.SIG_IGN); print('CHILD',os.getpid(),flush=True); time.sleep(30)"
            process, master, _slave, _original = self.spawn(root, code)
            output = bytearray()
            def child_ready():
                output.extend(self.read_available(master))
                return re.search(rb'CHILD (\d+)', output)
            match = wait_until(child_ready)
            child = int(match.group(1))
            def cleanup_child():
                try: os.kill(child, signal.SIGKILL)
                except ProcessLookupError: pass
            self.addCleanup(cleanup_child)
            os.kill(process.pid, signal.SIGTERM)
            process.wait(timeout=3)

    def test_resize_is_forwarded(self):
        with tempfile.TemporaryDirectory() as root:
            code = "import os,time; print('READY',flush=True); time.sleep(.7); print('SIZE',os.get_terminal_size().columns,os.get_terminal_size().lines,flush=True)"
            process, master, slave, original = self.spawn(root, code)
            output = bytearray()
            def ready():
                output.extend(self.read_available(master))
                return b'READY' in output
            wait_until(ready)
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 42, 110, 0, 0))
            os.kill(process.pid, signal.SIGWINCH)
            self.assertEqual(process.wait(timeout=5), 0)
            output.extend(self.read_available(master))
            self.assertIn(b'SIZE 110 42', output)

if __name__ == '__main__':
    unittest.main()
