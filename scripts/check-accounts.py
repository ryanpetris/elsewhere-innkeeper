#!/usr/bin/env python3
"""Exercise accounts, login expiry, and sharing in the Docker image."""
import concurrent.futures
import hashlib
import json
import os
import pty
import select
import signal
import resource
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import termios
import time
import uuid
from auth_fixture import Client, PASSWORD

with tempfile.TemporaryDirectory(prefix='innkeeper-accounts-') as temporary:
    data=Path(temporary)/'data'
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
    env=dict(os.environ,INNKEEPER_DATA_DIR=str(data),INNKEEPER_IN_DOCKER='0',INNKEEPER_DOCKER_CONTAINER='',INNKEEPER_DOCKER_NETWORK='',INNKEEPER_TLS='1',INNKEEPER_TLS_CERT='',INNKEEPER_TLS_KEY='',INNKEEPER_LISTEN=f'127.0.0.1:{port}')
    env.pop('INNKEEPER_LOCAL_ELSEWHERE',None)
    log=tempfile.TemporaryFile()
    process=subprocess.Popen([os.environ.get('INNKEEPER_BINARY','elsewhere-innkeeper')],env=env,stdout=log,stderr=log)
    def db():
        connection=sqlite3.connect(data/'state.sqlite3');connection.execute('PRAGMA foreign_keys=ON');return connection
    a=Client(f'https://127.0.0.1:{port}')
    try:
        for _ in range(100):
            try:
                assert a.api('/setup')['required'];break
            except (ConnectionError,OSError):time.sleep(.1)
        assert a.request('/me')[0]==401
        assert a.request('/setup','POST',dict(username='custom',display_name='First',password=PASSWORD),{'Origin':'https://foreign.invalid'})[0]==403
        candidates=[Client(a.origin),Client(a.origin)]
        with concurrent.futures.ThreadPoolExecutor() as pool:
            results=list(pool.map(lambda c:c.request('/setup','POST',dict(username='fixture',display_name='First',password=PASSWORD)),candidates))
        assert sorted(r[0] for r in results)==[201,409],results
        a=candidates[next(i for i,r in enumerate(results) if r[0]==201)]
        admin=a.api('/me')['user'];assert uuid.UUID(admin['id']).version==4
        assert not a.api('/setup')['required']
        # Configured setup refuses requests before even validating or hashing a password.
        assert a.request('/setup','POST',dict(username='other',display_name='Other',password='short'))[0]==409
        assert a.request('/setup','POST',dict(username='other',display_name='Other',password=PASSWORD))[0]==409
        cookie=results[next(i for i,r in enumerate(results) if r[0]==201)][1]
        cookie=next(v for k,v in cookie.items() if k.lower()=='set-cookie')
        assert all(v in cookie.lower() for v in ('httponly','secure','samesite=strict','path=/api'))
        assert 'set-cookie' not in {k.lower() for k in a.request('/me')[1]}
        u=a.api('/users','POST',dict(username='Person',display_name='A person',password=PASSWORD))['user']
        assert u['username']=='person' and 'password_hash' not in u
        assert a.request('/users','POST',dict(username='PERSON',display_name='Duplicate',password=PASSWORD))[0]==409
        assert a.request('/users','POST',dict(username='email',display_name='Email',password=PASSWORD,email='not-accepted'))[0]==400
        with db() as conn:
            hashes=[r[0] for r in conn.execute('SELECT password_hash FROM users')]
        assert len(set(hashes))==2 and all(h.startswith('$argon2id$v=19$m=19456,t=2,p=1$') for h in hashes)
        disabled=a.api('/users','POST',dict(username='disabled',display_name='Disabled',password=PASSWORD))['user']
        a.api('/users/'+disabled['id'],'PATCH',{'enabled':False})
        a.api('/users/'+disabled['id']+'/password','PUT',{'password':'reset while disabled password'})
        a.api('/users/'+disabled['id'],'PATCH',{'enabled':True})
        recovered=Client(a.origin);recovered.login('disabled','reset while disabled password')
        recovered.api('/me/password','PUT',dict(current_password='reset while disabled password',password='self service replacement password'))
        assert recovered.request('/me')[0]==401
        assert a.request('/users/'+str(uuid.uuid4())+'/password','PUT',{'password':PASSWORD})[0]==404
        viewer=Client(a.origin);viewer.login('person')
        assert viewer.request('/users')[0]==403
        assert viewer.request('/me','PATCH',{'username':'changed'})[0]==400
        assert viewer.api('/me','PATCH',{'display_name':'New display'})['user']['display_name']=='New display'
        assert a.request('/users/'+admin['id'],'PATCH',{'role':'user'})[0]==409
        assert a.request('/users/'+admin['id'],'DELETE')[0]==409
        assert a.request('/users','POST',{}, {'X-Innkeeper-CSRF':'bad'})[0]==403
        session=a.api('/sessions','POST',dict(name='Machine',distribution='debian',packages=[]))
        sid=session['id'];assert session['access_role']=='manager'
        assert not viewer.api('/sessions')['sessions']
        assert viewer.request(f'/sessions/{sid}/logs')[0]==404
        a.api(f'/sessions/{sid}/access/{u["id"]}','PUT',{'role':'viewer'})
        visible=viewer.api('/sessions')['sessions'][0]
        assert visible['access_role']=='viewer' and 'docker_args' not in visible and 'startup_command' not in visible
        for action in ('start','stop','relaunch','upgrade'):
            assert viewer.request(f'/sessions/{sid}/{action}','POST')[0]==403
        assert viewer.request(f'/sessions/{sid}/access')[0]==403
        assert viewer.request('/sessions','POST',dict(name='Denied',distribution='debian',packages=[],docker_args=['--cap-add=SYS_ADMIN']))[0]==403
        # A role change retires mismatched credentials atomically. Restoring grants never unmarks them.
        token_id=str(uuid.uuid4())
        with db() as conn:
            conn.execute("INSERT INTO instance_tokens VALUES(?,?,'user',?, ?,0)",(sid,token_id,u['id'],'a'*64))
            for p in ('audio.listen','clipboard.read','desktop.view'):
                conn.execute('INSERT INTO instance_token_permissions VALUES(?,?,?)',(sid,token_id,p))
        a.api(f'/sessions/{sid}/access/{u["id"]}','PUT',{'role':'interactive'})
        with db() as conn: assert conn.execute('SELECT revoked FROM instance_tokens WHERE token_id=?',(token_id,)).fetchone()[0]==1
        a.api(f'/sessions/{sid}/access/{u["id"]}','PUT',{'role':'viewer'})
        with db() as conn: assert conn.execute('SELECT revoked FROM instance_tokens WHERE token_id=?',(token_id,)).fetchone()[0]==1
        a.api(f'/sessions/{sid}/access/{u["id"]}','PUT',{'role':'manager'})
        with db() as conn: conn.execute("UPDATE sessions SET status='stopped' WHERE id=?", [sid])
        settings = dict(name='Edited',screen_size=None,kiosk=False,software_encoding=True,startup_command='',packages=[],docker_args=['--cap-drop=NET_RAW'],gpu_access=False,gpu_id=None)
        assert a.request(f'/sessions/{sid}/settings','PUT',settings)[0]==200
        assert viewer.request(f'/sessions/{sid}/settings','PUT',dict(settings,name='Manager edit'))[0]==200
        assert viewer.request(f'/sessions/{sid}/settings','PUT',dict(settings,docker_args=[]))[0]==403
        assert a.request(f'/sessions/{sid}/settings','PUT',dict(settings,distribution='arch'))[0]==400
        a.api(f'/sessions/{sid}/access/{u["id"]}','PUT',{'role':'viewer'})
        a.api('/users/'+u['id'],'PATCH',{'username':'renamed'})
        assert viewer.api('/me')['user']['id']==u['id']
        # Force the current login into its renewal window, then race two tabs.
        cookie_id=viewer.cookie.split('=',1)[1];digest=hashlib.sha256(cookie_id.encode()).digest()
        with db() as conn: conn.execute('UPDATE login_sessions SET expires_at_unix_seconds=?,expires_at_nanosecond=123456789 WHERE secret_hash=?',(int(time.time())+3600,digest))
        second=Client(a.origin);second.cookie=viewer.cookie;second.csrf=viewer.csrf
        with concurrent.futures.ThreadPoolExecutor() as pool: renewals=list(pool.map(lambda c:c.request('/session/renew','POST'),[viewer,second]))
        assert all(r[0]==200 for r in renewals),renewals
        assert abs(renewals[0][2]['session_expires_at_ms']-renewals[1][2]['session_expires_at_ms'])<1
        assert sum(any(k.lower()=='set-cookie' for k in r[1]) for r in renewals)==1
        before=viewer.api('/me')['session_expires_at_ms'];assert viewer.api('/session/renew','POST')['session_expires_at_ms']==before
        a.api('/users/'+u['id']+'/password','PUT',{'password':'a completely different password'})
        assert viewer.request('/me')[0]==401 and second.request('/session/renew','POST')[0]==401
        viewer.login('renamed','a completely different password')
        a.api('/users/'+u['id'],'DELETE')
        assert viewer.request('/me')[0]==401
        with db() as conn:
            assert conn.execute('SELECT user_id,revoked FROM instance_tokens WHERE token_id=?',(token_id,)).fetchone()==(None,1)
            assert conn.execute('SELECT count(*) FROM sessions WHERE id=?',(sid,)).fetchone()[0]==1
        # Ordinary requests retain the exact stored deadline.
        with db() as conn: before=list(conn.execute('SELECT secret_hash,expires_at_unix_seconds,expires_at_nanosecond FROM login_sessions ORDER BY secret_hash'))
        for _ in range(5): a.api('/me');a.api('/sessions')
        with db() as conn: assert before==list(conn.execute('SELECT secret_hash,expires_at_unix_seconds,expires_at_nanosecond FROM login_sessions ORDER BY secret_hash'))
        second_admin=a.api('/users','POST',dict(username='second-admin',display_name='Second',password=PASSWORD,role='administrator'))['user']
        other=Client(a.origin);other.login('second-admin')
        with concurrent.futures.ThreadPoolExecutor() as pool:
            responses=list(pool.map(lambda pair:pair[0].request('/users/'+pair[1],'PATCH',{'role':'user'}),[(a,admin['id']),(other,second_admin['id'])]))
        assert sorted(r[0] for r in responses)==[200,409],responses
        with db() as conn: assert conn.execute("SELECT count(*) FROM users WHERE role='administrator' AND enabled=1").fetchone()[0]==1
        binary=os.environ.get('INNKEEPER_BINARY','elsewhere-innkeeper')
        locked=subprocess.run([binary,'users','list'],env=env,capture_output=True)
        assert locked.returncode and b'Another Innkeeper' in locked.stderr
        wrong=Client(a.origin)
        assert [wrong.request('/login','POST',dict(username='missing',password=PASSWORD))[0] for _ in range(6)]==[401]*5+[429]
        a.api('/logout','POST');assert a.request('/me')[0]==401
        process.terminate();process.wait(timeout=10)
        listing=subprocess.run([binary,'users','list'],env=env,capture_output=True)
        assert listing.returncode==0 and admin['id'].encode() in listing.stdout
        # Recovery uses a controlling terminal and never echoes passwords.
        delay=Path(temporary)/'password-prompt-delay.so'
        subprocess.run(['cc','-shared','-fPIC',str(Path(__file__).with_name('password-prompt-delay.c')),'-o',str(delay),'-ldl'],check=True)
        with db() as conn:
            password_before=conn.execute('SELECT password_hash FROM users WHERE id=?',[second_admin['id']]).fetchone()
            sessions_before=list(conn.execute('SELECT * FROM login_sessions ORDER BY secret_hash'))
        cancellations=[(phase,sig) for sig in (signal.SIGINT,signal.SIGTERM,signal.SIGHUP,signal.SIGQUIT)
                       for phase in ('first','second','before-first','between')]
        cancellations += [('first','ctrl-c'),('second','ctrl-c')]
        for phase,sig in cancellations:
            release_read,release_write=os.pipe()
            prompt_read,prompt_write=os.pipe();os.set_inheritable(prompt_write,True)
            pid,terminal=pty.fork()
            if pid==0:
                resource.setrlimit(resource.RLIMIT_CORE,(0,0))
                os.close(release_write);os.close(prompt_read)
                os.read(release_read,1);os.close(release_read)
                child_env=dict(env)
                if phase in ('before-first','between'):
                    child_env.update(LD_PRELOAD=str(delay),INNKEEPER_PROMPT_READY_FD=str(prompt_write))
                os.execve(binary,[binary,'users','reset-password','--id',second_admin['id']],child_env)
            os.close(release_read);os.close(prompt_write)
            original=termios.tcgetattr(terminal)
            os.write(release_write,b'1');os.close(release_write)
            transcript=b'';cancelled=False;first_sent=False;deadline=time.monotonic()+10
            slave=None;exited=0;status=0
            def hold_terminal():
                return os.open(os.readlink(f'/proc/{pid}/fd/0'),os.O_RDWR|os.O_NONBLOCK|os.O_NOCTTY)
            secret=b'cancelled recovery password'
            try:
                while time.monotonic()<deadline:
                    readable=select.select([terminal,prompt_read],[],[],.1)[0]
                    if prompt_read in readable:
                        prompt=os.read(prompt_read,1)
                        if not cancelled and ((phase=='before-first' and prompt==b'N') or (phase=='between' and prompt==b'R')):
                            slave=hold_terminal();os.kill(pid,sig);cancelled=True
                    if terminal in readable:
                        try: chunk=os.read(terminal,4096)
                        except OSError: break
                        if not chunk: break
                        transcript+=chunk
                        if not cancelled and b'New password:' in transcript:
                            if phase=='first':
                                slave=hold_terminal()
                                os.write(terminal,secret)
                                if sig=='ctrl-c': os.write(terminal,b'\x03')
                                else: os.kill(pid,sig)
                                cancelled=True
                            elif phase in ('second','between') and not first_sent:
                                os.write(terminal,secret+b'\n');first_sent=True
                        if not cancelled and phase=='second' and b'Repeat password:' in transcript:
                            slave=hold_terminal()
                            os.write(terminal,secret)
                            if sig=='ctrl-c': os.write(terminal,b'\x03')
                            else: os.kill(pid,sig)
                            cancelled=True
                    if cancelled:
                        exited,status=os.waitpid(pid,os.WNOHANG)
                        if exited:
                            while select.select([terminal],[],[],0)[0]:
                                transcript+=os.read(terminal,4096)
                            break
                else: raise AssertionError(('Cancellation timed out',phase,sig))
                assert cancelled and termios.tcgetattr(terminal)==original,('Terminal not restored',phase,sig)
                assert secret not in transcript,transcript
                os.write(terminal,b'\n')
                assert select.select([slave],[],[],1)[0],('Terminal input unavailable',phase,sig)
                assert os.read(slave,4096)==b'\n',('Cancelled input reached shell',phase,sig)
            finally:
                if not exited:
                    exited,status=os.waitpid(pid,os.WNOHANG)
                    if not exited:
                        try: os.kill(pid,signal.SIGKILL)
                        except ProcessLookupError: pass
                        _,status=os.waitpid(pid,0)
                if slave is not None: os.close(slave)
                os.close(terminal);os.close(prompt_read)
            assert status!=0,(phase,sig,transcript)
            with db() as conn:
                assert conn.execute('SELECT password_hash FROM users WHERE id=?',[second_admin['id']]).fetchone()==password_before
                assert list(conn.execute('SELECT * FROM login_sessions ORDER BY secret_hash'))==sessions_before
        print('PASS: recovery cancellation restores the terminal and preserves password/login sessions',flush=True)
        for mode in ('mismatch', 'separate', 'together', 'delayed', 'editing', 'flow'):
            ready_read,ready_write=os.pipe()
            pid,terminal=pty.fork()
            if pid==0:
                os.close(ready_write);os.read(ready_read,1);os.close(ready_read)
                os.execve(binary,[binary,'users','reset-password','--id',second_admin['id']],dict(env,LD_PRELOAD=str(delay)) if mode=='delayed' else env)
            os.close(ready_read)
            original=termios.tcgetattr(terminal)
            os.write(ready_write,b'1');os.close(ready_write)
            transcript=b'';sent=0;deadline=time.monotonic()+10
            try:
                while time.monotonic()<deadline:
                    if select.select([terminal],[],[],.1)[0]:
                        try: chunk=os.read(terminal,4096)
                        except OSError: break
                        if not chunk:break
                        transcript+=chunk
                        if sent==0 and b'New password:' in transcript:
                            assert not termios.tcgetattr(terminal)[3] & (termios.ECHO | termios.ECHONL),(mode,transcript)
                            if mode=='flow': os.write(terminal,b'\x13')
                            os.write(terminal,b'local recovery passworX'+original[6][termios.VERASE]+b'd\n' if mode=='editing' else b'local recovery password\n');sent=1
                            if mode=='flow':
                                time.sleep(.15)
                                os.write(terminal,b'\x11')
                            if mode=='together':
                                os.write(terminal,b'local recovery password\n');sent=2
                        if sent==1 and b'Repeat password:' in transcript:
                            assert not termios.tcgetattr(terminal)[3] & (termios.ECHO | termios.ECHONL),(mode,transcript)
                            os.write(terminal,b'different recovery password\n' if mode=='mismatch' else b'local recovery password\n');sent=2
                else:
                    os.kill(pid,9);raise AssertionError('Recovery prompt timed out')
                assert termios.tcgetattr(terminal)==original
            finally:
                os.close(terminal)
                _,status=os.waitpid(pid,0)
            assert (status!=0 if mode=='mismatch' else status==0),transcript
            assert sent==2 and b'local recovery password' not in transcript and b'different recovery password' not in transcript,transcript
            assert b'Repeat password:' in transcript,transcript
            if mode=='mismatch': assert b'Passwords differ' in transcript,transcript
        with db() as conn: assert conn.execute('SELECT count(*) FROM login_sessions WHERE user_id=?',[second_admin['id']]).fetchone()[0]==0
        process=subprocess.Popen([binary],env=env,stdout=log,stderr=log)
        for _ in range(100):
            try:
                if other.request('/setup')[0]==200:break
            except (ConnectionError,OSError):pass
            time.sleep(.1)
        assert other.request('/me')[0]==401
        other.login('second-admin','local recovery password')
        print('PASS: setup races, UUID accounts, salted hashes, CSRF, roles, sharing, renewal, revocation markers, password resets, logout, login limits')
    finally:
        process.terminate();process.wait(timeout=10)
        log.seek(0)
        if process.returncode not in (0,-15): print(log.read().decode())
