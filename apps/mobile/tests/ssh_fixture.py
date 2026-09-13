"""Local-only SSH fixture. It never executes commands against the developer's files."""
import asyncio
import json
import os
from pathlib import Path
import secrets
import socket
import asyncssh
from aiohttp import web

ROOT = Path(__file__).resolve().parents[1] / '.build/fixtures'
ROOT.mkdir(parents=True, exist_ok=True)
os.chmod(ROOT, 0o700)
port_file = ROOT / 'ports.json'
if port_file.exists():
    PORTS = json.loads(port_file.read_text())
else:
    while True:
        first = secrets.randbelow(28000) + 15000
        sockets = []
        try:
            for port in range(first, first + 7):
                s = socket.socket(); sockets.append(s); s.bind(('127.0.0.1', port))
            break
        except OSError:
            continue
        finally:
            for s in sockets: s.close()
    PORTS = dict(zip(['ssh','http','adb','console','device','grpc','metrics'], range(first,first+7)))
    port_file.write_text(json.dumps(PORTS))

host_key = asyncssh.generate_private_key('ssh-ed25519')
client_key = asyncssh.generate_private_key('ssh-ed25519')
(ROOT/'client_key').write_bytes(client_key.export_private_key('openssh'))
os.chmod(ROOT/'client_key',0o600)
(ROOT/'fingerprint').write_text(host_key.get_fingerprint())
encrypted_key = client_key.export_private_key('openssh', passphrase='fixture-key-passphrase', cipher_name='aes256-ctr', rounds=16).decode()
(ROOT/'encrypted_key').write_text(encrypted_key)
os.chmod(ROOT/'encrypted_key', 0o600)
(ROOT/'ios.json').write_text(json.dumps({'port':PORTS['ssh'], 'httpPort':PORTS['http'], 'fingerprint':host_key.get_fingerprint(), 'privateKey':client_key.export_private_key('openssh').decode(), 'encryptedKey':encrypted_key}))
os.chmod(ROOT/'ios.json', 0o600)

class FixtureSSH(asyncssh.SSHServer):
    def begin_auth(self, username): return True
    def password_auth_supported(self): return True
    def public_key_auth_supported(self): return True
    def validate_password(self,username,password): return username=='fixture' and password=='fixture-only-password'
    def validate_public_key(self,username,key): return username=='fixture' and key==client_key.convert_to_public()
    def connection_requested(self,dest_host,dest_port,orig_host,orig_port):
        return dest_host=='127.0.0.1' and dest_port==PORTS['http']

async def command(process):
    if process.command=='uname -s': process.stdout.write('Linux\n')
    else: process.stdout.write(json.dumps({'port':PORTS['http'],'password':'fixture-web-password','pid':1}))
    process.exit(0)

async def echo(request):
    ws=web.WebSocketResponse();await ws.prepare(request)
    async for msg in ws:
        if msg.type==web.WSMsgType.TEXT: await ws.send_str(msg.data)
    return ws

async def main():
    app=web.Application()
    app.router.add_get('/api/mode',lambda r:web.json_response({'requirePairing':False,'fixture':True}))
    app.router.add_post('/api/login',lambda r:web.json_response({'token':'fixture-token'}))
    app.router.add_get('/echo',echo)
    runner=web.AppRunner(app);await runner.setup();await web.TCPSite(runner,'127.0.0.1',PORTS['http']).start()
    server=await asyncssh.create_server(FixtureSSH,'127.0.0.1',PORTS['ssh'],server_host_keys=[host_key],process_factory=command)
    print('Local fixture ready; ports persisted in .build/fixtures/ports.json',flush=True)
    await server.wait_closed()

asyncio.run(main())
