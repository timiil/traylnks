import importlib.util
import json
import pathlib
import os
import subprocess
import sys
from unittest.mock import patch
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('agent', pathlib.Path(__file__).with_name('traylnks_agent.py'))
agent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agent)

class AgentTests(unittest.TestCase):
    def test_frames_survive_every_chunk_boundary_without_becoming_output(self):
        event = {'version': 1, 'phase': 'completed', 'session_id': 'one', 'seq': 2}
        wire = agent.frame('secret', event)
        text = '中文\x1b[31mOK\x1b[0m'.encode()
        for split in range(len(wire) + 1):
            events = []
            parser = agent.StreamParser('secret', events.append)
            output = parser.feed(text + wire[:split]) + parser.feed(wire[split:] + b'!')
            self.assertEqual(output, text + b'!')
            self.assertEqual(events, [event])

    def test_wrong_token_is_discarded_and_osc_csi_are_not_activity(self):
        events = []
        parser = agent.StreamParser('secret', events.append)
        self.assertEqual(parser.feed(agent.frame('wrong', {'version': 1})), b'')
        self.assertEqual(events, [])
        parser.feed(b'\x1b]2;title\x07\x1b[?25h')
        self.assertFalse(parser.activity)
        parser.feed(b'actual text')
        self.assertTrue(parser.activity)

    def test_header_short_flash_then_static_and_fresh_title_is_preserved(self):
        title = agent.Header('project')
        title.control = {'enabled': True, 'marker': '✓', 'updated_at_ms': 1000, 'flash_until_ms': 3000}
        self.assertIn('· project', title.render(1500).decode())
        self.assertIn('✓ project', title.render(3000).decode())
        title.base = 'new project'
        self.assertIn('✓ new project', title.render(3000).decode())
        self.assertNotIn('✓', title.render(7000).decode())

    def test_hooks_do_not_store_prompts_or_idle_notifications(self):
        state = agent.State('one')
        state.hook('codex', {'hook_event_name': 'SessionStart', 'session_id': 'thread'})
        state.hook('codex', {'hook_event_name': 'UserPromptSubmit', 'prompt': 'SECRET'})
        run = state.data['run_id']
        state.hook('codex', {'hook_event_name': 'Stop'})
        seq = state.data['seq']
        self.assertEqual(state.data['phase'], 'completed')
        state.hook('codex', {'hook_event_name': 'Stop'})
        state.hook('codex', {'hook_event_name': 'Notification', 'notification_type': 'idle_prompt'})
        state.hook('codex', {'hook_event_name': 'SubagentStop'})
        self.assertEqual(state.data['seq'], seq)
        self.assertEqual(state.data['run_id'], run)
        self.assertNotIn('SECRET', json.dumps(state.data))
        state.hook('codex', {'hook_event_name': 'UserPromptSubmit'})
        self.assertNotEqual(state.data['run_id'], run)

    def test_permissions_and_terminal_errors_are_distinct(self):
        state = agent.State('one')
        state.hook('claude', {'hook_event_name': 'UserPromptSubmit', 'session_id': 's'})
        state.hook('claude', {'hook_event_name': 'PermissionRequest'})
        self.assertEqual(state.data['phase'], 'needs_input')
        state.hook('claude', {'hook_event_name': 'PreToolUse'})
        self.assertEqual(state.data['phase'], 'running')
        state.hook('claude', {'hook_event_name': 'StopFailure', 'error': 'rate_limit'})
        self.assertEqual(state.data['phase'], 'error')

    def test_install_respects_codex_home_and_private_permissions(self):
        with tempfile.TemporaryDirectory() as root:
            with patch.dict(os.environ, {'CODEX_HOME': root + '/profile', 'HOME': root + '/fallback'}):
                actual = agent.install_hooks('codex')
            self.assertEqual(actual, str(pathlib.Path(root) / 'profile/hooks.json'))
            self.assertEqual(os.stat(actual).st_mode & 0o077, 0)

    def test_old_session_cannot_complete_or_clear_new_session(self):
        state = agent.State('one')
        state.hook('codex', {'hook_event_name': 'UserPromptSubmit', 'session_id': 'old'})
        state.hook('codex', {'hook_event_name': 'UserPromptSubmit', 'session_id': 'new'})
        run = state.data['run_id']
        for name in ('Stop', 'SessionEnd', 'Interrupt', 'PreToolUse'):
            state.hook('codex', {'hook_event_name': name, 'session_id': 'old'})
            self.assertEqual(state.data['phase'], 'running')
            self.assertEqual(state.data['run_id'], run)
        state.hook('claude', {'hook_event_name': 'Stop', 'session_id': 'new'})
        self.assertEqual(state.data['phase'], 'running')

    def test_nested_start_and_compact_do_not_replace_parent_turn(self):
        state = agent.State('one')
        state.hook('codex', {'hook_event_name': 'SessionStart', 'session_id': 'parent'})
        state.hook('codex', {'hook_event_name': 'UserPromptSubmit', 'session_id': 'parent'})
        run = state.data['run_id']
        state.hook('codex', {'hook_event_name': 'SessionStart', 'session_id': 'parent', 'source': 'compact'})
        self.assertEqual(state.data['run_id'], run)
        state.hook('claude', {'hook_event_name': 'SessionStart', 'session_id': 'nested'})
        state.hook('claude', {'hook_event_name': 'UserPromptSubmit', 'session_id': 'nested'})
        state.hook('claude', {'hook_event_name': 'Stop', 'session_id': 'nested'})
        self.assertEqual(state.data['tool_session_id'], 'parent')
        self.assertEqual(state.data['phase'], 'running')
        self.assertEqual(state.data['run_id'], run)

    def test_new_cli_can_replace_a_crashed_owner(self):
        owner = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])
        try:
            state = agent.State('one')
            state.hook('codex', {'hook_event_name': 'SessionStart', 'session_id': 'old', 'process_id': owner.pid})
            state.hook('codex', {'hook_event_name': 'UserPromptSubmit', 'session_id': 'old', 'process_id': owner.pid})
            owner.terminate(); owner.wait(timeout=3)
            state.hook('claude', {'hook_event_name': 'SessionStart', 'session_id': 'new', 'process_id': os.getpid()})
            state.hook('claude', {'hook_event_name': 'UserPromptSubmit', 'session_id': 'new', 'process_id': os.getpid()})
            self.assertEqual(state.data['tool'], 'claude')
            self.assertEqual(state.data['tool_session_id'], 'new')
        finally:
            if owner.poll() is None: owner.kill(); owner.wait()

    def test_ssh_attempt_has_identity_and_failure_does_not_require_ai_hook(self):
        state = agent.State('one')
        state.hook('', {'hook_event_name': 'SSHStart', 'connection_id': 'connection-one'})
        self.assertEqual(state.data['tool'], 'ssh')
        self.assertTrue(state.data['run_id'])
        state.hook('', {'hook_event_name': 'SSHExit', 'connection_id': 'connection-one', 'exit_code': 255})
        self.assertEqual(state.data['phase'], 'error')
        self.assertEqual(state.data['tool'], 'ssh')

    def test_remote_heartbeats_are_independent_of_event_sequence(self):
        state = agent.State('one')
        state.hook('', {'hook_event_name': 'SSHStart', 'connection_id': 'conn'})
        event = dict(version=1, relay_id='relay', connection_id='conn', seq=2, tool='codex',
                     phase='completed', run_id='turn', tool_session_id='s', host='server')
        state.remote(event, now=1)
        state.remote(event, now=25)
        state.expire_remote(now=40)
        self.assertTrue(state.data['observer_available'])
        state.expire_remote(now=56)
        self.assertFalse(state.data['observer_available'])
        state.remote(event, now=57)
        self.assertTrue(state.data['observer_available'])
        self.assertEqual(state.data['phase'], 'completed')
        state.hook('', {'hook_event_name': 'SSHExit', 'connection_id': 'conn', 'exit_code': 255})
        state.remote(event, now=60)
        self.assertEqual(state.data['phase'], 'error')

    def test_hook_installation_merges_and_is_idempotent(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / 'settings.json'
            path.write_text(json.dumps({'permissions': {'allow': ['Read']}, 'hooks': {'Stop': [{'hooks': [{'type': 'command', 'command': 'existing'}]}]}}))
            agent.install_hooks('claude', path)
            first = json.loads(path.read_text())
            agent.install_hooks('claude', path)
            self.assertEqual(json.loads(path.read_text()), first)
            self.assertEqual(first['permissions']['allow'], ['Read'])
            self.assertEqual(first['hooks']['Stop'][0]['hooks'][0]['command'], 'existing')
            self.assertTrue(path.with_suffix('.json.traylnks-backup').exists())

if __name__ == '__main__':
    unittest.main()
