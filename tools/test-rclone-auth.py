#!/usr/bin/env python3
"""Exercise Rclone browser OAuth without Google, a browser, or live credentials.

Linux: python3 tools/test-rclone-auth.py /path/to/rclone
Uses a temporary config and mock OAuth server. Rclone's callback port (53682)
must be free. An xdg-open stub follows only loopback URLs.
"""
import http.client
import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import urllib.parse


class OAuthServer(http.server.BaseHTTPRequestHandler):
    exchanges = 0

    def log_message(self, *_):
        pass

    def do_GET(self):
        params = urllib.parse.parse_qs(urllib.parse.urlsplit(self.path).query)
        callback = params.get('redirect_uri', [''])[0]
        if callback not in ('http://127.0.0.1:53682/', 'http://localhost:53682/') or not params.get('state'):
            self.send_error(400, 'Invalid mock OAuth callback')
            return
        query = urllib.parse.urlencode({'code': 'synthetic-code', 'state': params['state'][0]})
        self.send_response(302)
        # Use a fixed loopback destination, never request text in the header.
        self.send_header('Location', 'http://127.0.0.1:53682/?' + query)
        self.end_headers()

    def do_POST(self):
        self.rfile.read(int(self.headers.get('Content-Length', 0)))
        type(self).exchanges += 1
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps({
            'access_token': 'synthetic-access', 'refresh_token': 'synthetic-refresh',
            'token_type': 'Bearer', 'expires_in': 3600,
        }).encode())


def main():
    executable = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix='boreal-oauth-test-') as directory:
        folder = Path(directory)
        config = folder / 'rclone.conf'
        config.write_text('[extra-storage]\ntype = local\n')
        config.chmod(0o600)
        opener = folder / 'xdg-open'
        opener.write_text(f'#!{sys.executable}\n' + '''import sys, urllib.request, urllib.parse
class LoopbackOnly(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        assert urllib.parse.urlsplit(newurl).hostname in ('127.0.0.1', 'localhost')
        return super().redirect_request(req, fp, code, msg, headers, newurl)
assert urllib.parse.urlsplit(sys.argv[1]).hostname == '127.0.0.1'
urllib.request.build_opener(urllib.request.ProxyHandler({}), LoopbackOnly()).open(sys.argv[1], timeout=10).read()
''')
        opener.chmod(0o700)
        # Avoid inherited Rclone configuration overrides and proxy settings.
        env = {key: value for key, value in os.environ.items()
               if not key.startswith('RCLONE_') and 'proxy' not in key.lower()}
        env['PATH'] = str(folder) + os.pathsep + os.environ.get('PATH', '')
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), OAuthServer)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        endpoint = f'http://127.0.0.1:{server.server_port}'

        def run(args, success=True):
            result = subprocess.run([executable, *args, '--config', str(config)],
                                    stdin=subprocess.DEVNULL, capture_output=True,
                                    env=env, timeout=15)
            if success and result.returncode:
                # Never print raw OAuth output, even from this synthetic fixture.
                diagnostic = result.stderr.decode(errors='replace')
                reason = 'callback port occupied' if 'address already in use' in diagnostic else 'unexpected Rclone failure'
                raise AssertionError(f'{reason}; exit {result.returncode}')
            return result

        try:
            # Exercise callbacks that hostname-only validation would miss.
            for callback in ('http://127.0.0.1:53682/\r\nX-Injected: yes',
                             'http://127.0.0.1:53682/?extra=1',
                             'http://127.0.0.1:9999/', 'https://example.com/', ''):
                connection = http.client.HTTPConnection('127.0.0.1', server.server_port)
                connection.request('GET', '/authorize?' + urllib.parse.urlencode({
                    'redirect_uri': callback, 'state': 'synthetic-state'}))
                response = connection.getresponse()
                assert response.status == 400
                assert response.getheader('Location') is None
                assert response.getheader('X-Injected') is None
                response.read()
                connection.close()
            for name, scope in [('my-drive-ro', 'drive.readonly'), ('my-drive-rw', 'drive')]:
                options = ['client_id', 'synthetic-app', 'client_secret', 'synthetic-secret',
                           'scope', scope, 'auth_url', endpoint + '/authorize', 'token_url', endpoint + '/token']
                run(['config', 'create', name, 'drive', *options, '--auto-confirm', '--obscure'])
                run(['config', 'update', name, 'token', '', 'config_refresh_token', 'false', '--non-interactive'])
                failed = run(['config', 'reconnect', name + ':'], success=False)
                assert failed.returncode and b'Failed to read line: EOF' in failed.stderr
                run(['config', 'reconnect', name + ':', '--auto-confirm'])
                # Reconnect also suppresses the prompt to replace a present token.
                run(['config', 'reconnect', name + ':', '--auto-confirm'])
            data = json.loads(run(['config', 'dump']).stdout)
            assert data['extra-storage'] == {'type': 'local'}
            for name, scope in [('my-drive-ro', 'drive.readonly'), ('my-drive-rw', 'drive')]:
                assert data[name]['scope'] == scope
                assert json.loads(data[name]['token'])['refresh_token'] == 'synthetic-refresh'
                assert not data[name].get('team_drive')
            assert OAuthServer.exchanges == 6
            print('PASS: both scopes created, missing and existing tokens reconnected, EOF reproduced without auto-confirm; unrelated connection preserved. Six mock OAuth exchanges, no Google access.')
        finally:
            server.shutdown()
            server.server_close()
            worker.join()


if __name__ == '__main__':
    main()
