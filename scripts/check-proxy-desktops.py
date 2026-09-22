#!/usr/bin/env python3
"""Run fresh real-package desktops in the Docker rig; optionally wait for browser checks."""
import json
import math
import struct
import wave
import os
from pathlib import Path
import ssl
import subprocess
import time
import urllib.request
import uuid
from sqlite_fixture import seed, database, docker_ports
from auth_fixture import Client, PASSWORD
import shutil

work = Path('/work')
for marker in ('browser.json', 'browser-done', 'capabilities'):
    (work / marker).unlink(missing_ok=True)
tone=work/'tone.wav'
with wave.open(str(tone),'wb') as wav:
    wav.setparams((1,2,48000,0,'NONE','not compressed'))
    wav.writeframes(b''.join(struct.pack('<h',int(8000*math.sin(2*math.pi*440*i/48000))) for i in range(48000)))
data = work / 'data'
data.mkdir(exist_ok=True)
# Each invocation has fresh accounts and port reservations, with cached packages retained.
for name in ('state.sqlite3', 'state.sqlite3-wal', 'state.sqlite3-shm'):
    (data / name).unlink(missing_ok=True)
# Reserve host ports already in use by unrelated containers in this disposable database.
reserved = [dict(id=str(uuid.uuid4()), name='Reserved fixture port', distribution='debian', packages=[],
                 port=port, status='failed', stage='download', error=None) for port in docker_ports()]
seed(data, reserved, str(uuid.uuid4()))
cert, key = work / 'cert.pem', work / 'key.pem'
subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1','-subj','/CN=localhost',
                '-keyout',str(key),'-out',str(cert)],check=True,capture_output=True)
env = dict(os.environ, INNKEEPER_DATA_DIR=str(data), INNKEEPER_LISTEN='0.0.0.0:29301',
           INNKEEPER_TLS_CERT=str(cert), INNKEEPER_TLS_KEY=str(key))
if os.environ.get('PROXY_DEFAULT_NVIDIA_RUNTIME') == '1' or os.environ.get('PROXY_CHECK_FAILURES') == '1':
    # Override runtime defaults and capabilities for our creates only.
    shim = work / 'default-runtime-bin'
    shim.mkdir(exist_ok=True)
    real_docker = shutil.which('docker')
    (shim / 'docker').write_text('''#!/usr/bin/env python3
import os, sys
from pathlib import Path
args = sys.argv[1:]
if args[0] == 'create':
    if os.environ.get('PROXY_DEFAULT_NVIDIA_RUNTIME') == '1': args.insert(1, '--runtime=nvidia')
    capabilities = Path('/work/capabilities')
    if capabilities.exists():
        args = ['--env=NVIDIA_DRIVER_CAPABILITIES='+capabilities.read_text() if arg.startswith('--env=NVIDIA_DRIVER_CAPABILITIES=') else arg for arg in args]
os.execv('''+repr(real_docker)+''', ['''+repr(real_docker)+'''] + args)
''')
    (shim / 'docker').chmod(0o755)
    env['PATH'] = str(shim) + ':' + env['PATH']
# Exercise the Debian library directory with the real Arch-host driver. Overmounting
# the normal GBM directory leaves the injected files underneath untouched.
alternate_gbm = os.environ.get('PROXY_GBM_LAYOUT') == 'debian'
if alternate_gbm:
    recipes = work / 'assets'
    shutil.copytree('/usr/share/elsewhere-innkeeper/sessions', recipes / 'sessions', dirs_exist_ok=True)
    script = recipes / 'sessions/gpu.sh'
    hook = r'''if [ "$INNKEEPER_GPU_DRIVER" = nvidia ] && [ -c /dev/nvidia-modeset ]; then
    allocator=$(ldconfig -p | awk '$1 == "libnvidia-allocator.so.1" && /x86-64/ {print $NF; exit}')
    test -n "$allocator"
    mkdir -p /usr/lib/gbm /usr/lib/x86_64-linux-gnu/gbm
    mount -t tmpfs tmpfs /usr/lib/gbm
    if [ -d /usr/lib64/gbm ] && [ "$(readlink -f /usr/lib64/gbm)" != /usr/lib/gbm ]; then mount -t tmpfs tmpfs /usr/lib64/gbm; fi
    ln -sf "$allocator" /usr/lib/x86_64-linux-gnu/gbm/nvidia-drm_gbm.so
fi
'''
    script.write_text(script.read_text().replace('. /opt/innkeeper/gpu-settings.sh\n', '. /opt/innkeeper/gpu-settings.sh\n' + hook))
    env['INNKEEPER_ASSETS_DIR'] = str(recipes)
if Path('/local/manifest.json').exists(): env['INNKEEPER_LOCAL_ELSEWHERE']='/local/manifest.json'
if Path('/packages').exists():
    for distro, filename in [('arch','elsewhere-0.7.3-1-x86_64.pkg.tar.zst'),('debian','elsewhere_0.7.3-1_debian-13_amd64.deb')]:
        cache=data/'packages'/'0.7.3'/'x86_64'/distro;cache.mkdir(parents=True,exist_ok=True);shutil.copyfile(Path('/packages')/filename,cache/filename)
log = (work/'manager.log').open('w')
manager = subprocess.Popen(['elsewhere-innkeeper'],env=env,stdout=log,stderr=log)
context = ssl._create_unverified_context()
origin = 'https://127.0.0.1:29301'
created = []

account=Client(origin)
api=account.api

def wait(test, timeout=120):
    deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
        if manager.poll() is not None: raise AssertionError('Manager exited')
        if test(): return
        time.sleep(1)
    raise AssertionError('Timed out')

def state(sid):
    return next(s for s in api('/sessions')['sessions'] if s['id']==sid)

try:
    def listening():
        try:return account.request('/setup')[0]==200
        except (ConnectionError,OSError):return False
    wait(listening)
    account.setup()
    viewer_account=Client(origin)
    viewer_user=api('/users','POST',dict(username='viewer',display_name='Viewer',password=PASSWORD))['user']
    viewer_account.login('viewer')
    browser=[]
    gpu_checks = os.environ.get('PROXY_CHECK_GPU') == '1'
    gpu_access = os.environ.get('PROXY_GPU_ACCESS') == '1'
    if gpu_checks:
        devices = api('/sessions')['gpus']
        if os.environ.get('PROXY_CHECK_DISCOVERY_ERROR') == '1':
            assert any('renderD999' in error and 'sysfs' in error for error in api('/sessions')['gpu_errors'])
        available = bool(devices)
        selected = next((g for g in devices if g['driver'] == os.environ['PROXY_GPU_DRIVER']), None) if os.environ.get('PROXY_GPU_DRIVER') else next(iter(devices), None)
        if gpu_access: assert selected, devices
        for options in (dict(gpu_access=True,gpu_id='not-a-device'), dict(gpu_access=False,gpu_id='not-a-device')):
            status, _, _ = account.request('/sessions', 'POST', dict(name='Invalid GPU',distribution='arch',packages=[], **options))
            assert status == 400
        assert api('/sessions')['gpu_available'] == available
        if gpu_access and selected['driver'] == 'nvidia':
            tools = work / 'no-runtime-bin'
            tools.mkdir(exist_ok=True)
            fake_docker = tools / 'docker'
            fake_docker.write_text("#!/bin/sh\nif [ \"$1\" = info ] && [ \"${3:-}\" = '{{json .Runtimes}}' ]; then printf '{}\\n'; else exec /usr/bin/docker \"$@\"; fi\n")
            fake_docker.chmod(0o755)
            negative_env = dict(env, PATH=str(tools) + ':' + env.get('PATH', '/usr/bin'),
                                INNKEEPER_DATA_DIR=str(work/'no-runtime-data'), INNKEEPER_LISTEN='127.0.0.1:29302',
                                INNKEEPER_IN_DOCKER='0', INNKEEPER_DOCKER_CONTAINER='', INNKEEPER_DOCKER_NETWORK='')
            shutil.rmtree(work/'no-runtime-data', ignore_errors=True)
            negative = subprocess.Popen(['elsewhere-innkeeper'],env=negative_env,stdout=log,stderr=log)
            try:
                client = Client('https://127.0.0.1:29302')
                def negative_ready():
                    assert negative.poll() is None
                    try: return client.request('/setup')[0] == 200
                    except (ConnectionError, OSError): return False
                wait(negative_ready)
                client.setup()
                status, _, body = client.request('/sessions', 'POST', dict(name='Missing runtime',distribution='arch',packages=[],gpu_access=True,gpu_id=selected['id']))
                assert status == 400 and 'nvidia runtime' in str(body), body
                assert client.api('/sessions')['sessions'] == []
            finally:
                negative.terminate()
                negative.wait(timeout=10)
        if not available:
            assert account.request('/sessions', 'POST', dict(name='Unavailable GPU', distribution='ubuntu', packages=[], gpu_access=True))[0] == 400
    for distro in os.environ.get('PROXY_DISTROS','arch,debian,ubuntu').split(','):
        options = dict(gpu_access=gpu_access, gpu_id=selected['id'] if gpu_access else None, software_encoding=False) if gpu_checks else {}
        if alternate_gbm:
            assert distro == 'arch' and gpu_access and selected['driver'] == 'nvidia'
            options['docker_args'] = ['--cap-add=SYS_ADMIN', '--security-opt=seccomp=unconfined', '--security-opt=apparmor=unconfined']
        extra = ['vulkan-tools', 'mesa-utils'] if gpu_checks else []
        glx32 = gpu_checks and gpu_access and selected['driver'] == 'nvidia' and distro == 'arch' and os.environ.get('PROXY_CHECK_GLX32') == '1'
        if glx32: extra += ['gcc', 'lib32-glibc', 'lib32-libglvnd', 'lib32-libx11']
        sid=api('/sessions','POST',dict(name='Proxy '+distro,distribution=distro,packages=['foot'] + extra,startup_command='foot',screen_size={'width':640,'height':480}, **options))['id']
        created.append(sid)
        def ready():
            s=state(sid)
            if s['status']=='failed':
                print(api('/sessions/'+sid+'/logs')['text'],flush=True)
                raise AssertionError(s)
            return s['status']=='running'
        wait(ready,600)
        name='innkeeper-'+sid
        info=json.loads(subprocess.check_output(['docker','inspect',name]))[0]
        if gpu_checks:
            def check_gpu(software):
                current = json.loads(subprocess.check_output(['docker','inspect',name]))[0]
                devices = current['HostConfig']['Devices'] or []
                assert any(d['PathInContainer']=='/dev/dri' for d in devices) == gpu_access, devices
                assert current['Id'] == info['Id']
                assert state(sid)['gpu_access'] == gpu_access
                assert state(sid)['software_encoding'] == software
                command = subprocess.check_output(['docker','top',name,'-eo','pid,args'],text=True)
                desktop = next(line.split(None,1)[1] for line in command.splitlines()[1:] if len(line.split(None,1)) == 2 and line.split(None,1)[1].startswith('elsewhere '))
                assert ('--software-encoding' in desktop.split()) == software, desktop
                logs = api('/sessions/'+sid+'/logs')['text']
                encoders = [line for line in logs.splitlines() if 'verified video encoders' in line]
                backend = 'Software' if software else ('Nvenc' if selected['driver'] == 'nvidia' else 'Vaapi')
                assert encoders and backend in encoders[-1], logs
                if gpu_access:
                    assert state(sid)['gpu'] == selected
                    assert '--render-node ' + selected['node'] in desktop
                    if selected['driver'] == 'nvidia':
                        assert current['HostConfig']['Runtime'] == 'nvidia'
                        assert 'NVIDIA_VISIBLE_DEVICES=all' in current['Config']['Env']
                        renderers = [line for line in logs.splitlines() if 'GL Renderer:' in line]
                        assert renderers and 'NVIDIA' in renderers[-1], logs
                else:
                    assert '--render-node none' in desktop
                    assert 'NVIDIA_VISIBLE_DEVICES=void' in current['Config']['Env']
                    if os.environ.get('PROXY_DEFAULT_NVIDIA_RUNTIME') == '1':
                        assert current['HostConfig']['Runtime'] == 'nvidia'
                return current
            check_gpu(not gpu_access)
            groups = subprocess.check_output(['docker','exec',name,'id','-G','elsewhere'],text=True).split()
            assert '0' not in groups, groups
            if gpu_access:
                subprocess.run(['docker','exec','--user','elsewhere',name,'sh','-c',
                                'test -r '+selected['node']+' && test -w '+selected['node']],check=True)
            else:
                subprocess.run(['docker','exec',name,'sh','-ec', 'for device in /dev/dri/renderD* /dev/nvidia* /dev/nvidia-caps/*; do if [ -c "$device" ]; then echo "Unexpected GPU device: $device"; exit 1; fi; done'],check=True)
            probe = subprocess.run(['docker','exec','--user','elsewhere','-e',
                         'XDG_RUNTIME_DIR=/tmp/runtime-elsewhere',name,'vulkaninfo','--summary'],text=True,capture_output=True)
            vulkan = probe.stdout + probe.stderr
            assert probe.returncode == 0 or (not gpu_access and probe.returncode == 1 and 'ERROR_INITIALIZATION_FAILED' in vulkan), vulkan
            assert ('PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU' in vulkan or 'PHYSICAL_DEVICE_TYPE_DISCRETE_GPU' in vulkan) == gpu_access, vulkan
            if gpu_access and selected['driver'] == 'nvidia':
                if glx32:
                    # Exercise package-owned GL dispatch files after runtime injection.
                    subprocess.run(['docker','exec',name,'pacman','-S','--noconfirm',
                                    'mesa','lib32-mesa','libglvnd','lib32-libglvnd'],check=True)
                xenv = subprocess.check_output(['docker','exec','--user','elsewhere',name,'sh','-c',
                    'for p in /proc/[0-9]*/comm; do if [ "$(cat "$p" 2>/dev/null)" = Xwayland ]; then tr "\\0" "\\n" < "${p%/comm}/environ"; fi; done'],text=True)
                assert 'GBM_BACKENDS_PATH=' in xenv, xenv
                if alternate_gbm: assert 'GBM_BACKENDS_PATH=/usr/lib/x86_64-linux-gnu/gbm' in xenv, xenv
                print(distro+': '+next(line for line in xenv.splitlines() if line.startswith('GBM_BACKENDS_PATH=')),flush=True)
                glx = subprocess.check_output(['docker','exec','--user','elsewhere','-e','DISPLAY=:0',name,'glxinfo','-B'],text=True)
                assert 'direct rendering: Yes' in glx and 'NVIDIA' in glx, glx
                egl = subprocess.check_output(['docker','exec','--user','elsewhere','-e','XDG_RUNTIME_DIR=/tmp/runtime-elsewhere','-e','WAYLAND_DISPLAY=elsewhere',name,'eglinfo','-B','-p','wayland'],text=True)
                assert 'EGL vendor string: NVIDIA' in egl and 'OpenGL core profile renderer: NVIDIA' in egl, egl
                print(distro+': NVIDIA Xwayland wrapper environment, direct GLX and Wayland EGL passed',flush=True)
                if glx32:
                    subprocess.run(['docker','cp','/check-glx32.c',name+':/tmp/check-glx32.c'],check=True)
                    subprocess.run(['docker','exec',name,'gcc','-m32','/tmp/check-glx32.c','-ldl','-o','/tmp/check-glx32'],check=True)
                    subprocess.run(['docker','exec','--user','elsewhere','-e','DISPLAY=:0',name,'/tmp/check-glx32'],check=True)
            for software, action in ((True,'relaunch'),(False,'start'),(True,'relaunch'),(os.environ.get('PROXY_BROWSER_SOFTWARE') == '1','relaunch')):
                desired = {key:state(sid)[key] for key in ('name','screen_size','kiosk','startup_command','packages','docker_args','gpu_access','gpu_id')}
                desired['software_encoding'] = software
                api('/sessions/'+sid+'/settings','PUT',desired)
                if action == 'start': api('/sessions/'+sid+'/stop','POST')
                api('/sessions/'+sid+'/'+action,'POST')
                wait(ready)
                check_gpu(software or not gpu_access)
                assert not state(sid)['settings_pending']
            if gpu_access:
                for broken in (dict(selected, id='missing-gpu'), dict(selected, minor=999)):
                    api('/sessions/'+sid+'/stop','POST')
                    with database(data) as db:
                        configured = json.loads(db.execute('SELECT configured FROM sessions WHERE id=?', [sid]).fetchone()[0])
                        configured['gpu'] = broken
                        db.execute('UPDATE sessions SET gpu=?,configured=? WHERE id=?', [json.dumps(broken),json.dumps(configured),sid])
                    previous = info['Id']
                    api('/sessions/'+sid+'/start','POST')
                    wait(ready)
                    info = json.loads(subprocess.check_output(['docker','inspect',name]))[0]
                    assert info['Id'] != previous
                    check_gpu(os.environ.get('PROXY_BROWSER_SOFTWARE') == '1')
                print(distro+': missing GPU and changed device mapping select compatible hardware and recreate',flush=True)
                if selected['driver'] != 'nvidia':
                    desired = {key:state(sid)[key] for key in ('name','screen_size','kiosk','startup_command','packages','docker_args','gpu_access','gpu_id','software_encoding')}
                    for access in (False, True):
                        desired.update(gpu_access=access, gpu_id=selected['id'] if access else None, software_encoding=not access)
                        api('/sessions/'+sid+'/settings','PUT',desired)
                        previous = info['Id']
                        api('/sessions/'+sid+'/relaunch','POST')
                        wait(ready)
                        info = json.loads(subprocess.check_output(['docker','inspect',name]))[0]
                        assert info['Id'] != previous
                        gpu_access = access
                        check_gpu(not access)
                    print(distro+': GPU to software to GPU rendering preserves the installation',flush=True)
            print(distro+': GPU devices, Vulkan client access, encoding flags/logs and Start/Relaunch passed',flush=True)
        if older := os.environ.get('PROXY_UPGRADE_FROM'):
            asset=(f'elsewhere-{older}-1-x86_64.pkg.tar.zst' if distro=='arch' else
                   f'elsewhere_{older}-1_{dict(debian="debian-13",ubuntu="ubuntu-26.04")[distro]}_amd64.deb')
            archive=work/asset
            subprocess.run(['curl','--fail','--location','--output',str(archive),
                            f'https://github.com/ryanpetris/elsewhere/releases/download/v{older}/{asset}'],check=True)
            installed='/tmp/'+asset
            subprocess.run(['docker','cp',str(archive),name+':'+installed],check=True)
            install=(['pacman','-U','--noconfirm',installed] if distro=='arch' else
                     ['apt-get','install','-y','--allow-downgrades',installed])
            subprocess.run(['docker','exec','-e','DEBIAN_FRONTEND=noninteractive',name,*install],check=True)
            wait(lambda:state(sid)['installed_version']==older+'-1')
            api('/sessions/'+sid+'/upgrade','POST')
            wait(lambda:state(sid)['status']=='stopped',600)
            assert state(sid)['installed_version']==state(sid)['expected_version']+'-1'
            api('/sessions/'+sid+'/start','POST')
            wait(ready)
            assert json.loads(subprocess.check_output(['docker','inspect',name]))[0]['Id']==info['Id']
            print(distro+': real package upgrade from '+older+' passed',flush=True)
        bindings=info['HostConfig']['PortBindings']
        port=state(sid)['port']
        launch = subprocess.check_output(['docker', 'top', name, '-eo', 'pid,args'], text=True)
        commands = [line.split(None, 1)[1] for line in launch.splitlines()[1:] if len(line.split(None, 1)) == 2]
        desktop = next(line for line in commands if line.startswith('elsewhere ') and '--url-prefix' in line)
        assert ('--rtc-addr' in desktop) == bool(env.get('INNKEEPER_RTC_ADDR'))
        assert '--rtc-port ' + str(port) in desktop
        assert '19443/tcp' not in bindings
        assert bindings[str(port)+'/udp']==[{'HostIp':'0.0.0.0','HostPort':str(port)}]
        api('/sessions/'+sid+'/access/'+viewer_user['id'],'PUT',{'role':'viewer'})
        preview=account.request('/sessions/'+sid+'/preview?width=320')
        assert preview[0]==200 and preview[2].startswith(b'\x89PNG'), preview
        with database(data) as db: assert db.execute("SELECT count(*) FROM instance_tokens WHERE kind='user' AND session_id=?",[sid]).fetchone()[0]==0
        with database(data) as db: internal=db.execute("SELECT secret FROM instance_tokens WHERE kind='internal' AND revoked=0 AND session_id=?",[sid]).fetchone()[0]
        assert internal not in json.dumps(api('/sessions')) and internal not in api('/sessions/'+sid+'/logs')['text']
        link=account.connect(sid)
        viewer_link=viewer_account.connect(sid)
        viewer_token=viewer_link.split('#token=')[1]
        assert account.connect(sid)==link
        with urllib.request.urlopen(origin+link.split('#')[0],context=context) as response:
            assert ('<base href="/e/'+sid+'/">').encode() in response.read()
        req=urllib.request.Request(origin+'/e/'+sid+'/api/screenshot.png',headers={'Authorization':'Bearer '+viewer_token})
        with urllib.request.urlopen(req,context=context,timeout=20) as response: assert response.read().startswith(b'\x89PNG')
        req=urllib.request.Request(origin+'/e/'+sid+'/api/me',headers={'Authorization':'Bearer '+viewer_token})
        with urllib.request.urlopen(req,context=context,timeout=20) as response:
            identity=json.load(response)
            assert set(identity['permissions'])=={'audio.listen','clipboard.read','desktop.view'} and identity['metadata']['expires_at_ms'] is None
        def bearer_status(token,path='/api/me'):
            request=urllib.request.Request(origin+'/e/'+sid+path,headers={'Authorization':'Bearer '+token})
            try:
                with urllib.request.urlopen(request,context=context,timeout=20) as response:return response.status
            except urllib.error.HTTPError as error:return error.code
        assert bearer_status(viewer_token,'/api/tokens')==403
        # Access changes revoke remotely without creating a replacement.
        api('/sessions/'+sid+'/access/'+viewer_user['id'],'PUT',{'role':'interactive'})
        wait(lambda:bearer_status(viewer_token)==401)
        def user_token_removed():
            with database(data) as db:
                return db.execute("SELECT count(*) FROM instance_tokens WHERE kind='user' AND user_id=? AND session_id=?",[viewer_user['id'],sid]).fetchone()[0]==0
        wait(user_token_removed)
        replacement=viewer_account.connect(sid).split('#token=')[1]
        assert replacement!=viewer_token and bearer_status(replacement)==200
        api('/sessions/'+sid+'/access/'+viewer_user['id'],'PUT',{'role':'viewer'})
        wait(lambda:bearer_status(replacement)==401)
        wait(user_token_removed)
        viewer_token=viewer_account.connect(sid).split('#token=')[1]
        with database(data) as db: before_stop=list(db.execute('SELECT token_id,revoked FROM instance_tokens WHERE session_id=? ORDER BY token_id',[sid]))
        api('/sessions/'+sid+'/stop','POST')
        assert viewer_account.request('/sessions/'+sid+'/connect')[0]==409
        with database(data) as db: assert before_stop==list(db.execute('SELECT token_id,revoked FROM instance_tokens WHERE session_id=? ORDER BY token_id',[sid]))
        api('/sessions/'+sid+'/start','POST')
        wait(ready)
        assert account.connect(sid)==link and viewer_account.connect(sid).endswith(viewer_token)
        api('/sessions/'+sid+'/relaunch','POST')
        wait(ready)
        assert json.loads(subprocess.check_output(['docker','inspect',name]))[0]['Id']==info['Id']
        assert account.connect(sid)==link
        subprocess.run(['docker','cp',str(tone),name+':/tmp/tone.wav'],check=True)
        browser.append(dict(id=sid,distribution=distro,link=link,viewer=viewer_token,port=port))
        print(distro+': package installation, plain HTTP/prefix, private mapping, public UDP, tokens, screenshots, preview, Stop/Start and Relaunch passed',flush=True)
    if os.environ.get('PROXY_CHECK_FAILURES') == '1':
        assert gpu_access and selected['driver'] == 'nvidia'
        for capabilities, message in [('compute,video,utility', 'NVIDIA control or modeset device is missing'),
                                      ('compute,graphics,utility,display,compat32', 'no usable FFmpeg video encoder')]:
            (work/'capabilities').write_text(capabilities)
            failed_id=api('/sessions','POST',dict(name='Missing NVIDIA prerequisite',distribution='arch',packages=[],
                gpu_access=True,gpu_id=selected['id'],software_encoding=False,
                docker_args=options.get('docker_args', [])))['id']
            created.append(failed_id)
            wait(lambda:state(failed_id)['status']=='failed',180)
            logs=api('/sessions/'+failed_id+'/logs')['text']
            assert message in logs, logs
            assert state(failed_id)['gpu'] == selected and not state(failed_id)['software_encoding']
            (work/'capabilities').unlink()
            print('Missing NVIDIA capabilities '+capabilities+': useful failure, selection and encoding preference retained',flush=True)
    (work/'browser.json').write_text(json.dumps(browser))
    if os.environ.get('PROXY_WAIT_BROWSER')=='1':
        wait(lambda:(work/'browser-done').exists(),600)
        assert (work/'browser-done').read_text()=='PASS'
finally:
    for sid in created:
        try: api('/sessions/'+sid,'DELETE')
        except Exception:
            subprocess.run(['docker','rm','-f','innkeeper-'+sid],capture_output=True)
            subprocess.run(['docker','volume','rm','innkeeper-'+sid+'-data'],capture_output=True)
    for sid in created:
        images = subprocess.check_output(['docker','image','ls','-q','--filter','label=io.innkeeper.snapshot=true','--filter','label=io.innkeeper.session='+sid], text=True).splitlines()
        for image in dict.fromkeys(images):
            subprocess.run(['docker','image','rm',image],capture_output=True)
    manager.terminate()
    manager.wait(timeout=10)
    print((work/'manager.log').read_text())
