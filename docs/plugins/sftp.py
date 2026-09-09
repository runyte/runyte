# SPDX-License-Identifier: MPL-2.0
"""SFTP browser and provider editor. Run with --config /absolute/profile.json."""
import argparse
import json
import sys

from application import PluginError
from remote_application import MAX_ROWS, MAX_MODEL_BYTES, RemoteApplication


class SftpApplication(RemoteApplication):
    def __init__(self, transport, provider, plugin_id='sftp', app=None):
        super().__init__(transport, provider, plugin_id, app,
                         protocol_label='SFTP', atomic_replace=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', required=True, help='JSON SFTP connection profile')
    parser.add_argument('--plugin-id', default='sftp', help='ID in Runyte plugin configuration')
    options = parser.parse_args()
    try:
        from sftp_transport import SftpTransport
        from remote_provider import RemoteProvider
        with open(options.config, 'rb') as profile:
            raw = profile.read(64 * 1024 + 1)
        if len(raw) > 64 * 1024:
            raise PluginError('limit_exceeded', 'SFTP profile exceeds 64 KiB')
        transport = SftpTransport(json.loads(raw))
        SftpApplication(transport, RemoteProvider(transport, name='remote'), options.plugin_id).app.run()
    except (ImportError, OSError, ValueError, PluginError):
        # Local paths, library exceptions and connection details stay off stdout.
        print('SFTP startup failed; check the profile and installed dependencies.', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
