"""Run native transport checks against the running local fixture and a dedicated emulator."""
import json
import os
from pathlib import Path
import subprocess
ROOT=Path(__file__).resolve().parents[1]
fixture=ROOT/'.build/fixtures'
ports=json.loads((fixture/'ports.json').read_text())
sdk=Path(os.environ.get('ANDROID_HOME',str(Path.home()/'Library/Android/sdk')))
adb=[str(sdk/'platform-tools/adb'),'-P',str(ports['adb']),'-s','emulator-'+str(ports['console'])]
def run(*args,**kwargs):return subprocess.run(adb+list(args),check=True,**kwargs)
run('install','-r',str(ROOT/'android/app/build/outputs/apk/debug/app-debug.apk'))
run('install','-r',str(ROOT/'android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk'))
run('shell','run-as','com.velaterm.mobile','mkdir','-p','files')
for source,target in [('client_key','fixture_key'),('encrypted_key','fixture_encrypted_key')]:
    with (fixture/source).open('rb') as key:
        run('shell','run-as','com.velaterm.mobile','sh','-c','"cat > files/'+target+'"',stdin=key)
result=run('shell','am','instrument','-w','-r','-e','class','com.velaterm.mobile.SshIntegrationTest','-e','sshPort',str(ports['ssh']),'-e','httpPort',str(ports['http']),'-e','fingerprint',(fixture/'fingerprint').read_text().strip(),'com.velaterm.mobile.test/androidx.test.runner.AndroidJUnitRunner',capture_output=True,text=True)
print(result.stdout)
if 'OK (1 test)' not in result.stdout:raise SystemExit('Native SSH integration failed')
