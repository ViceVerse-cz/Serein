"""Offline regression for a stalled compatibility peer; no audio or network."""
import sys
import unittest
from unittest.mock import patch

import run


class PeerDeadlineTest(unittest.TestCase):
    def test_stalled_reply_names_operation_and_stops_both_peers(self):
        peer_type = run.Peer
        peers = []

        def stalled_peer(command, name):
            # Wait for EOF without replying; no sleeps or wall-clock assertions.
            peer = peer_type([sys.executable, '-c', 'import sys; sys.stdin.read()'],
                             name, reply_timeout=0.05)
            peers.append(peer)
            return peer

        with patch.object(run, 'Peer', side_effect=stalled_peer):
            with self.assertRaisesRegex(TimeoutError, 'legacy peer timed out during external'):
                run.exchange('unused-legacy', 'unused-current', True)
        self.assertEqual(len(peers), 2)
        for peer in peers:
            self.assertIsNotNone(peer.process.poll())
            self.assertTrue(peer.process.stdin.closed and peer.process.stdout.closed)

    def test_large_request_to_nonreading_peer_keeps_timeout_error_during_cleanup(self):
        peer = run.Peer([sys.executable, '-c', 'import threading; threading.Event().wait()'],
                        'nonreading', reply_timeout=0.05)
        try:
            with self.assertRaisesRegex(TimeoutError, 'nonreading peer timed out during encrypt'):
                peer.request('encrypt', payload='x' * 60000)
        finally:
            peer.stop()
            peer.close()


if __name__ == '__main__':
    unittest.main()
