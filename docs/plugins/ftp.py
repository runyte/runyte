# SPDX-License-Identifier: MPL-2.0
"""FTP/FTPS browser and provider editor. Run with --config /absolute/profile.json."""
import argparse
import json
import sys

from application import PluginError
from remote_application import RemoteApplication


class FtpApplication(RemoteApplication):
    def __init__(self, transport, provider, plugin_id='ftp', app=None):
        if transport.protocol not in ('ftps', 'ftp'):
            raise PluginError('invalid_argument', 'FTP transport must be ftps or explicitly ftp')
        label = 'FTPS' if transport.protocol == 'ftps' else 'FTP (unencrypted)'
        super().__init__(transport, provider, plugin_id, app,
                         protocol_label=label, atomic_replace=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', required=True, help='JSON FTP/FTPS connection profile')
    parser.add_argument('--plugin-id', default='ftp', help='ID in Runyte plugin configuration')
    options = parser.parse_args()
    try:
        # The FTP path uses only the standard library and never imports Paramiko.
        from ftp_transport import FtpTransport
        from remote_provider import RemoteProvider
        with open(options.config, 'rb') as profile:
            raw = profile.read(64 * 1024 + 1)
        if len(raw) > 64 * 1024:
            raise PluginError('limit_exceeded', 'FTP profile exceeds 64 KiB')
        transport = FtpTransport(json.loads(raw))
        FtpApplication(transport, RemoteProvider(transport, name='remote'), options.plugin_id).app.run()
    except (ImportError, OSError, ValueError, PluginError):
        print('FTP/FTPS startup failed; check the profile and credential file.', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
