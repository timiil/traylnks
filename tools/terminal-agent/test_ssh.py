"""Optional real OpenSSH loopback test: uv run --with asyncssh python -m unittest discover -s tools/terminal-agent -p test_ssh.py -v"""
import asyncio
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import unittest
try:
    import asyncssh
except ImportError:
    asyncssh = None

AGENT = Path(__file__).with_name('traylnks_agent.py').resolve()


@unittest.skipUnless(asyncssh and shutil.which('ssh'), 'optional asyncssh and OpenSSH client required')
class SshTests(unittest.IsolatedAsyncioTestCase):
    async def test_real_ssh_relay_and_abrupt_disconnect(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            remote = root / 'remote'
            script = remote / '.local/share/traylnks/traylnks_agent.py'
            script.parent.mkdir(parents=True)
            shutil.copy(AGENT, script)
            fake_shell = remote / 'shell.py'
            fake_shell.write_text('''#!/usr/bin/env python3
import json,subprocess,sys,time
from pathlib import Path
bridge=Path.home()/'.local/share/traylnks/traylnks_agent.py'
for event in ('SessionStart','UserPromptSubmit'):
 subprocess.run([sys.executable,str(bridge),'hook','--tool','codex'],input=json.dumps({'hook_event_name':event,'session_id':'remote-session'}),text=True,stdout=subprocess.DEVNULL)
print('REMOTE_中文',flush=True)
time.sleep(.5)
subprocess.run([sys.executable,str(bridge),'hook','--tool','codex'],input=json.dumps({'hook_event_name':'Stop','session_id':'remote-session'}),text=True,stdout=subprocess.DEVNULL)
time.sleep(30)
''', encoding='utf-8')
            fake_shell.chmod(0o700)
            client_key = asyncssh.generate_private_key('ssh-ed25519')
            key_path = root / 'client-key'
            key_path.write_bytes(client_key.export_private_key())
            key_path.chmod(0o600)
            connections = []
            local_processes = []
            class Server(asyncssh.SSHServer):
                def connection_made(self, connection): connections.append(connection)
                def begin_auth(self, username): return True
                def public_key_auth_supported(self): return True
                def validate_public_key(self, username, key):
                    return username == 'monitor-test' and key.export_public_key() == client_key.export_public_key()

            async def proxy(process):
                master, slave = pty.openpty()
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 90, 0, 0))
                environment = {**os.environ, 'HOME': str(remote), 'SHELL': str(fake_shell)}
                child = await asyncio.create_subprocess_exec('/bin/sh', '-c', process.command,
                    stdin=slave, stdout=slave, stderr=slave, env=environment)
                local_processes.append(child)
                os.close(slave)
                loop = asyncio.get_running_loop()
                queue = asyncio.Queue()
                def readable():
                    try: data = os.read(master, 65536)
                    except OSError: data = b''
                    queue.put_nowait(data)
                    if not data: loop.remove_reader(master)
                loop.add_reader(master, readable)
                async def input_stream():
                    try:
                        while True:
                            data = await process.stdin.read(65536)
                            if not data: return
                            os.write(master, data)
                    except (OSError, asyncssh.Error): pass
                reader = asyncio.create_task(input_stream())
                try:
                    while True:
                        data = await queue.get()
                        if not data: break
                        process.stdout.write(data)
                    await child.wait()
                    process.exit(child.returncode)
                except (OSError, asyncssh.Error, asyncio.CancelledError): pass
                finally:
                    reader.cancel()
                    loop.remove_reader(master)
                    os.close(master)
                    if child.returncode is None:
                        child.terminate()
                        try: await asyncio.wait_for(child.wait(), 2)
                        except asyncio.TimeoutError: child.kill(); await child.wait()

            server = await asyncssh.create_server(Server, '127.0.0.1', 0,
                server_host_keys=[asyncssh.generate_private_key('ssh-ed25519')],
                process_factory=proxy, encoding=None)
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 90, 0, 0))
            environment = {**os.environ, 'HOME': str(root / 'local'), 'SHELL': '/bin/sh'}
            environment.pop('ENV', None)
            environment.pop('TRAYLNKS_AGENT_SOCKET', None)
            command = [sys.executable, str(AGENT), 'ssh', '--state-dir', str(root / 'monitor'), '--',
                '-F', '/dev/null', '-p', str(server.get_port()), '-oStrictHostKeyChecking=no',
                '-oUserKnownHostsFile=/dev/null', '-oPreferredAuthentications=publickey',
                '-oIdentitiesOnly=yes', '-i', str(key_path), '-l', 'monitor-test',
                '-oIdentityAgent=none', '127.0.0.1']
            client = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave,
                                      env=environment, start_new_session=True)
            os.close(slave)
            output = bytearray()
            async def phase(expected):
                for _ in range(200):
                    while select.select([master], [], [], 0)[0]:
                        try: data = os.read(master, 65536)
                        except OSError: break
                        if not data: break
                        output.extend(data)
                    for path in (root / 'monitor/sessions').glob('*.json'):
                        data = json.loads(path.read_text())
                        if data['phase'] == expected: return data
                    await asyncio.sleep(.05)
                self.fail('SSH phase not reached: ' + expected + '; output=' + output.decode(errors='replace'))
            try:
                completed = await phase('completed')
                self.assertEqual(completed['tool_session_id'], 'remote-session')
                self.assertTrue(completed['observer_available'])
                self.assertIn('REMOTE_中文'.encode(), output)
                self.assertNotIn(b'777;TrayLnks;', output)
                for connection in connections: connection.abort()
                failed = await phase('error')
                self.assertIn('255', failed['reason'])
                self.assertEqual(failed['tool'], 'codex')
            finally:
                if client.poll() is None:
                    client.terminate()
                    await asyncio.to_thread(client.wait, 4)
                os.close(master)
                for connection in connections: connection.abort()
                server.close(); await server.wait_closed()
                for child in local_processes:
                    if child.returncode is None:
                        child.kill(); await child.wait()

if __name__ == '__main__': unittest.main()
