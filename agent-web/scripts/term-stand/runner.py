#!/usr/bin/env python3
"""Runner stand for the terminal e2e scenarios: listens on a unix socket and, per connection, starts `agentd runner`
with stdin and stdout on the accepted connection (what systemd does with `Accept=yes`).
Arguments: socket_path agentd_path uid project_dir. Dev-VM stand only, not part of the product."""
import os
import socket
import subprocess
import sys

sock_path, agentd, uid, project = sys.argv[1:5]
if os.path.exists(sock_path):
    os.unlink(sock_path)
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(sock_path)
os.chmod(sock_path, 0o600)
srv.listen(16)
while True:
    conn, _ = srv.accept()
    subprocess.Popen(
        [agentd, "runner", "--allow-uid", uid, "--allow-dir", project],
        stdin=conn,
        stdout=conn,
        stderr=subprocess.DEVNULL,
        close_fds=True,
    )
    conn.close()
