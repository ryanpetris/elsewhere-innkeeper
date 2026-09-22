#!/usr/bin/env python3
"""Run in the Innkeeper Docker image with Python, zstd, OpenSSL and the Docker socket.

Uses tiny real Arch, Debian and Ubuntu packages and disposable sessions to check upgrades and settings.
"""
import concurrent.futures
import argparse
import secrets
import uuid
from auth_fixture import Client
import http.server
import json
from sqlite_fixture import database, settings as stored_settings, reject_updates
import os
from pathlib import Path
import shutil
import signal
import threading
import subprocess
import tempfile
import time
import tomllib
import urllib.request
import urllib.parse

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--local", action="store_true")
parser.add_argument("--distribution", choices=("arch", "debian", "ubuntu"))
options = parser.parse_args()
distributions = (options.distribution,) if options.distribution else ("arch", "debian", "ubuntu")


def run(*args):
    return subprocess.check_output(args, text=True).strip()


with tempfile.TemporaryDirectory(prefix="innkeeper-refresh-") as temporary:
    work = Path(temporary)
    data = work / "data"
    assets = work / "assets"
    recipes = assets / "sessions"
    recipes.mkdir(parents=True)
    source = Path("/usr/share/elsewhere-innkeeper/sessions")
    shutil.copytree(source, recipes, dirs_exist_ok=True)
    for distro in ("arch", "debian", "ubuntu"):
        (recipes / f"setup-{distro}.sh").write_text(
            "set -eu\nuseradd -m " + ("" if distro == "ubuntu" else "-u 1000 ") + "elsewhere\n"
        )
    (recipes / "packages.sh").write_text("exit 0\n")
    # Exercise the production launcher with a fixture session bus and desktop.
    permissions = ['apps.launch','audio.listen','broadcasts.manage','camera.send','clipboard.read','clipboard.write','commands.execute','desktop.control','desktop.view','dragdrop.upload','files.browse','files.download','files.manage','files.upload','microphone.send','tokens.manage','server.manage']
    inventories = {}
    class Ready(http.server.BaseHTTPRequestHandler):
        def reply(self, status, body=None):
            self.send_response(status); self.send_header('Content-Type','application/json'); self.end_headers()
            if body is not None: self.wfile.write(json.dumps(body).encode())
        def inventory(self):
            with database(data) as db:
                sid = db.execute('SELECT id FROM sessions WHERE port=?',[self.server.server_port]).fetchone()[0]
            entries = inventories.setdefault(sid,{})
            lines=run('docker','exec','innkeeper-'+sid,'cat','/home/elsewhere/.config/elsewhere/fixture-tokens').splitlines()
            for line in lines:
                token_id, secret = line.split()
                entries.setdefault(secret,dict(id=token_id,label='Admin',created_at_ms=0,expires_at_ms=None,permissions=permissions))
            return entries
        def authorized(self):
            if not self.headers.get('Authorization'): self.reply(401); return None
            try:
                entries=self.inventory(); secret=self.headers.get('Authorization','').removeprefix('Bearer ')
                if secret not in entries: self.reply(401);return None
                if (work/'not-ready').exists(): self.reply(503);return None
                return entries,secret
            except Exception: self.reply(503);return None
        def do_GET(self):
            auth=self.authorized()
            if auth is None:return
            entries,secret=auth
            if self.path.endswith('/api/me'):self.reply(200,dict(metadata=entries[secret],permissions=entries[secret]['permissions'],available_permissions=permissions,features={}))
            elif self.path.endswith('/api/tokens'):self.reply(200,dict(tokens=list(entries.values())))
            else:self.reply(200,[])
        def do_POST(self):
            auth=self.authorized()
            if auth is None:return
            entries,_=auth;body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            secret=secrets.token_hex(32);metadata=dict(id=str(uuid.uuid4()),label=body['label'],created_at_ms=0,expires_at_ms=body['expires_at_ms'],permissions=body['permissions']);entries[secret]=metadata
            self.reply(201,dict(token=secret,metadata=metadata))
        def do_DELETE(self):
            auth=self.authorized()
            if auth is None:return
            entries,_=auth;token_id=self.path.rsplit('/',1)[-1]
            for secret,meta in list(entries.items()):
                if meta['id']==token_id:del entries[secret];self.reply(204);return
            self.reply(404)
        def log_message(self,*args):pass
    for port in (19500, 19501, 19502):
        readiness = http.server.ThreadingHTTPServer(("127.0.0.1", port), Ready)
        threading.Thread(target=readiness.serve_forever, daemon=True).start()
    # Use Docker-assigned host ports so the rig can coexist with live sessions.
    tools = work / "bin"
    tools.mkdir()
    wrapper = tools / "docker"
    wrapper.write_text("""#!/usr/bin/python3
import os, sys
from pathlib import Path
args = sys.argv[1:]
if args[:1] == ['create'] or args[:2] == ['volume', 'create']:
    Path(__file__).with_name('resource-created').touch()
if args[:1] in (['image'], ['pull']) and Path(__file__).with_name('no-base').exists():
    Path(__file__).with_name('image-attempt').touch()
    sys.exit(1)
if args and args[0] in ('cp', 'start', 'stop', 'commit', 'rm') and Path(__file__).with_name('fail-' + args[0]).exists():
    sys.exit(1)
if args and args[0] == 'create':
    if Path(__file__).with_name('fail-replacement-create').exists(): sys.exit(1)
    # Model a daemon whose default logging driver is journald.
    if '--log-driver' not in args:
        args[1:1] = ['--log-driver', 'journald']
    if Path(__file__).with_name('fail-create').exists():
        args[args.index('--log-driver') + 1] = 'journald'
    args[1:1] = ['--env', 'INNKEEPER_SCREEN_SIZE=640x480', '--env', 'INNKEEPER_KIOSK=1', '--env', 'INNKEEPER_STARTUP_COMMAND=stale']
    for i, arg in enumerate(args):
        if arg == '-p':
            args[i + 1] = '127.0.0.1::' + args[i + 1].rsplit(':', 1)[1]
os.execv('/usr/bin/docker', ['docker', *args])
""")
    wrapper.chmod(0o755)
    version = tomllib.loads((Path(__file__).resolve().parent.parent / "Cargo.toml").read_text())["package"]["metadata"]["elsewhere"]["version"]
    local_mode = options.local
    if local_mode:
        version = "99.0.0.7.dirty"


    def package(distro, label, installed=None, depends=None):
        installed = installed or version
        root = work / f"package-{distro}"
        shutil.rmtree(root, ignore_errors=True)
        (root / "usr/bin").mkdir(parents=True)
        binary = root / "usr/bin/elsewhere"
        binary.write_text("""#!/bin/sh
set -eu
if [ "${1:-}" = token ]; then
    test "${2:-}" = create && test "${3:-}" = --admin
    if [ -f "$HOME/token-command-fails" ]; then echo credential-output-must-not-leak; echo credential-diagnostic-must-not-leak >&2; exit 1; fi
    mkdir -p "$HOME/.config/elsewhere"
    secret=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
    id=$(cat /proc/sys/kernel/random/uuid)
    printf '%s %s\n' "$id" "$secret" >> "$HOME/.config/elsewhere/fixture-tokens"
    printf '%s\n' "$secret"
    exit 0
fi
printf '%s\\0' "$@" > "$HOME/launch-args"
printf '%s\n' LABEL >> "$HOME/launches"
echo 'https://example.invalid/#token=opaque control+/=?%%:initial'
exec sleep 10000
""".replace("LABEL", label))
        (root / "usr/local/bin").mkdir(parents=True)
        bus = root / "usr/local/bin/dbus-daemon"
        bus.write_text("#!/bin/sh\necho unix:path=/tmp/fixture-bus\n")
        bus.chmod(0o755)
        binary.chmod(0o755)
        cache = data / "packages" / version / "x86_64" / distro
        cache.mkdir(parents=True, exist_ok=True)
        if distro == "arch":
            (root / ".PKGINFO").write_text(
                f"pkgname = elsewhere\npkgver = {installed}-1\npkgdesc = Refresh fixture\n"
                "arch = x86_64\nbuilddate = 0\nsize = 100\nlicense = MIT\n"
            )
            path = cache / f"elsewhere-{version}-1-x86_64.pkg.tar.zst"
            run("tar", "--zstd", "-cf", str(path), "-C", str(root), ".PKGINFO", "usr")
        else:
            (root / "DEBIAN").mkdir()
            (root / "DEBIAN/control").write_text(
                f"Package: elsewhere\nVersion: {installed}-1\nArchitecture: amd64\n"
                "Maintainer: Test <test@example.invalid>\nDescription: Refresh fixture\n"
                + (f"Depends: {depends}\n" if depends else "")
            )
            release = "debian-13" if distro == "debian" else "ubuntu-26.04"
            path = cache / f"elsewhere_{version}-1_{release}_amd64.deb"
            run("dpkg-deb", "--build", "--root-owner-group", str(root), str(path))
        return path

    # Pre-pull only the stock images used by our disposable containers.
    for image in ("archlinux:base", "debian:13-slim", "ubuntu:26.04"):
        run("docker", "pull", image)
    for distro in ("arch", "debian", "ubuntu"):
        package(distro, "first")
    env = dict(os.environ, PATH=f"{tools}:{os.environ['PATH']}",
               INNKEEPER_DATA_DIR=str(data), INNKEEPER_ASSETS_DIR=str(assets),
               INNKEEPER_LISTEN="127.0.0.1:29300", INNKEEPER_IN_DOCKER="0", INNKEEPER_RTC_ADDR="127.0.0.1")
    if local_mode:
        local = work / "local"
        generation = local / "build-fixture"
        generation.mkdir(parents=True)
        for distro in ("arch", "debian", "ubuntu"):
            archive = package(distro, "first")
            shutil.copyfile(archive, generation / archive.name)
        manifest = local / "manifest.json"
        manifest.write_text(json.dumps({"version": version, "directory": generation.name}))
        env["INNKEEPER_LOCAL_ELSEWHERE"] = str(manifest)
        curl = tools / "curl"
        curl.write_text("#!/bin/sh\ntouch " + str(work / "download-attempt") + "\nexit 1\n")
        curl.chmod(0o755)
    log = (work / "manager.log").open("w+")
    manager = subprocess.Popen(["elsewhere-innkeeper"], env=env, stdout=log, stderr=log)
    created = []

    account = Client('http://127.0.0.1:29300')
    api = account.api

    def wait(check, timeout=45):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if check():
                return
            time.sleep(0.2)
        raise AssertionError("Timed out waiting for " + str(check))

    def state(sid):
        return next(s for s in api("/sessions")["sessions"] if s["id"] == sid)

    def launched(sid, count):
        result = subprocess.run(
            ["docker", "exec", "innkeeper-" + sid, "cat", "/home/elsewhere/launches"],
            capture_output=True, text=True,
        )
        return result.returncode == 0 and len(result.stdout.splitlines()) == count

    def restart_manager():
        global manager
        manager.send_signal(signal.SIGINT)
        manager.wait(timeout=10)
        manager = subprocess.Popen(["elsewhere-innkeeper"], env=env, stdout=log, stderr=log)
        def online():
            try: return api("/sessions")
            except OSError: return False
        wait(online)

    def rejected_action(sid, action, code):
        try:
            api(f"/sessions/{sid}/{action}", "POST")
            raise AssertionError(f"Unexpectedly accepted {action}")
        except urllib.error.HTTPError as error:
            assert error.code == code, error.code
            return error.read().decode()

    docker_args = ["--security-opt=seccomp=unconfined", "--security-opt=apparmor=unconfined",
                   "--cap-add=SYS_ADMIN", "--cap-drop=NET_RAW"]

    def check_docker_args(sid, expected):
        assert state(sid)["docker_args"] == expected
        with database(data) as db:
            stored = [row["argument"] for row in db.execute(
                "SELECT argument FROM session_docker_args WHERE session_id = ? ORDER BY position", [sid])]
        assert stored == expected
        config = json.loads(run("docker", "inspect", "innkeeper-" + sid))[0]["HostConfig"]
        assert sorted(config["SecurityOpt"] or []) == sorted(
            arg.split("=", 1)[1] for arg in expected if arg.startswith("--security-opt=")), config
        for flag, field in (("--cap-add=", "CapAdd"), ("--cap-drop=", "CapDrop")):
            assert sorted(config[field] or []) == sorted(
                arg.split("=", 1)[1] for arg in expected if arg.startswith(flag)), config

    def rejected(path, method, body, status):
        try:
            api(path, method, body)
        except urllib.error.HTTPError as error:
            assert error.code == status, (error.code, error.read())
            return error.read().decode()
        raise AssertionError("Request unexpectedly succeeded")

    try:
        wait(lambda: (data / "state.sqlite3").exists())
        time.sleep(1)
        account.setup()
        for invalid in (["--privileged=true"], ["--name=override"], ["--network=host"],
                        ["--entrypoint=sh"], ["--security-opt", "seccomp=unconfined"],
                        ["--cap-add="], ["--cap-add=SYS_ADMIN\n"], ["--cap-add=SYS_ADMIN\r"],
                        ["--cap-add=SYS_ADMIN\0"], ["--cap-add=SYS_ADMIN"] * 65,
                        ["--security-opt=" + "x" * 4096]):
            error = rejected("/sessions", "POST", {"name": "Invalid options",
                             "distribution": "debian", "packages": [], "docker_args": invalid}, 400)
            assert "Docker" in error or "docker" in error, error
        for invalid in (None, "--cap-add=SYS_ADMIN", [42]):
            rejected("/sessions", "POST", {"name": "Invalid option type", "distribution": "debian",
                     "packages": [], "docker_args": invalid}, 400)
        assert api("/sessions")["sessions"] == []
        assert not (tools / "resource-created").exists()
        print("Invalid Docker options rejected before session or Docker resource creation", flush=True)
        (tools / "fail-create").touch()
        probe = api("/sessions", "POST", {"name": "Container creation failure", "distribution": "debian", "packages": []})["id"]
        created.append(probe)
        wait(lambda: state(probe)["status"] == "failed")
        cause = state(probe)["error"]
        assert isinstance(cause, str) and "unknown log opt" in cause and "journald" in cause, cause
        time.sleep(7)
        assert state(probe)["error"] == cause
        (tools / "fail-create").unlink()
        api(f"/sessions/{probe}", "DELETE")
        created.remove(probe)
        print("Container creation error survives status polling", flush=True)
        if local_mode:
            assert api("/sessions")["local_elsewhere"] is True
            for distro in distributions:
                sid = api("/sessions", "POST", {"name": "Local " + distro,
                          "distribution": distro, "packages": []})["id"]
                created.append(sid)
                wait(lambda: state(sid)["status"] == "running", timeout=90)
                check_docker_args(sid, [])
                logging = json.loads(run("docker", "inspect", "innkeeper-" + sid))[0]["HostConfig"]["LogConfig"]
                assert logging == {"Type": "json-file", "Config": {"max-size": "10m", "max-file": "3"}}, logging
                assert state(sid)["expected_version"] == version
                assert state(sid)["installed_version"] == version + "-1"
                assert state(sid)["version_status"] == "current"
                assert account.connect(sid)
                name = "innkeeper-" + sid
                for installed in (version, "99.0.0.8.dirty"):
                    if installed != version:
                        archive = package(distro, "different", installed)
                        destination = "/tmp/fixture." + ("pkg.tar.zst" if distro == "arch" else "deb")
                        run("docker", "cp", str(archive), name + ":" + destination)
                        command = ["pacman", "-U", "--noconfirm"] if distro == "arch" else ["dpkg", "-i"]
                        run("docker", "exec", name, *command, destination)
                        restart_manager()
                        wait(lambda: state(sid)["version_status"] == "unknown")
                    archive = package(distro, "reinstalled")
                    shutil.copyfile(archive, generation / archive.name)
                    before = run("docker", "exec", name, "cat", "/home/elsewhere/launches")
                    api(f"/sessions/{sid}/upgrade", "POST")
                    wait(lambda: state(sid)["status"] == "stopped", timeout=90)
                    assert state(sid)["installed_version"] == version + "-1"
                    payload = work / "installed-elsewhere"
                    run("docker", "cp", name + ":/usr/bin/elsewhere", str(payload))
                    assert "reinstalled" in payload.read_text()
                    api(f"/sessions/{sid}/start", "POST")
                    wait(lambda: state(sid)["status"] == "running")
                    assert run("docker", "exec", name, "cat", "/home/elsewhere/launches").splitlines() == before.splitlines() + ["reinstalled"]
                # Start uses the installed package even if the local artifact disappears.
                cached = package(distro, "first")
                artifact = generation / cached.name
                artifact.unlink()
                api(f"/sessions/{sid}/stop", "POST")
                api(f"/sessions/{sid}/start", "POST")
                wait(lambda: state(sid)["status"] == "running")
                missing = api("/sessions", "POST", {"name": "Missing package",
                              "distribution": distro, "packages": []})["id"]
                created.append(missing)
                wait(lambda: state(missing)["status"] == "failed")
                assert "Local Elsewhere package is missing" in api(f"/sessions/{missing}/logs")["text"]
                assert not (work / "download-attempt").exists()
                api(f"/sessions/{missing}", "DELETE")
                created.remove(missing)
                invalid = subprocess.run(["elsewhere-innkeeper"], env=dict(env, INNKEEPER_DATA_DIR=str(work / "invalid")),
                                         capture_output=True, text=True, timeout=10)
                assert invalid.returncode != 0 and "Local Elsewhere package is missing" in invalid.stderr
                shutil.copyfile(cached, artifact)
                api(f"/sessions/{sid}/stop", "POST")
                print(f"{distro}: local dirty package, same-version and incomparable reinstalls, token CLI, launch-only Start and missing-package failure passed", flush=True)
            manager.send_signal(signal.SIGINT)
            manager.wait(timeout=10)
            env.pop("INNKEEPER_LOCAL_ELSEWHERE")
            manager = subprocess.Popen(["elsewhere-innkeeper"], env=env, stdout=log, stderr=log)
            def normal_mode():
                try: return api("/sessions")["local_elsewhere"] is False
                except OSError: return False
            wait(normal_mode)
            pinned = tomllib.loads((Path(__file__).resolve().parent.parent / "Cargo.toml").read_text())["package"]["metadata"]["elsewhere"]["version"]
            assert all(s["expected_version"] == pinned for s in api("/sessions")["sessions"])
            print("Ordinary startup restores the Cargo release pin without changing installed packages", flush=True)
            version = pinned
            for sid in created:
                distro = state(sid)["distribution"]
                name = "innkeeper-" + sid
                identity = run("docker", "inspect", name, "--format", "{{.Id}}")
                package(distro, "pinned")
                api(f"/sessions/{sid}/upgrade", "POST")
                wait(lambda: state(sid)["status"] == "stopped", timeout=90)
                assert state(sid)["installed_version"] == pinned + "-1"
                assert run("docker", "inspect", name, "--format", "{{.Id}}") == identity
                payload = work / "installed-elsewhere"
                run("docker", "cp", name + ":/usr/bin/elsewhere", str(payload))
                assert "pinned" in payload.read_text()
                api(f"/sessions/{sid}/start", "POST")
                wait(lambda: state(sid)["status"] == "running")
                assert run("docker", "exec", name, "cat", "/home/elsewhere/launches").splitlines()[-1] == "pinned"
                print(f"{distro}: explicit installation returns the local build to the release pin", flush=True)
        for distro in (() if local_mode else distributions):
            # Creation failures retain their cause even when no container exists.
            cached = package(distro, "first")
            cached.unlink()
            prepare_script = (recipes / "prepare.sh").read_text()
            for script, cancelled in [("echo 'Fixture download failure'\nexit 1\n", False),
                                      ("echo 'Fixture waiting'\nexec sleep 60\n", True)]:
                (recipes / "prepare.sh").write_text(script)
                probe = api("/sessions", "POST", {"name": "Creation recovery", "distribution": distro, "packages": []})["id"]
                created.append(probe)
                wait(lambda: "Fixture" in api(f"/sessions/{probe}/logs")["text"])
                if cancelled:
                    api(f"/sessions/{probe}/stop", "POST")
                else:
                    wait(lambda: state(probe)["status"] == "failed")
                expected = state(probe)
                time.sleep(4)
                assert state(probe)["status"] == ("cancelled" if cancelled else "failed")
                assert state(probe)["error"] == expected["error"]
                api(f"/sessions/{probe}", "DELETE")
                created.remove(probe)
            (recipes / "prepare.sh").write_text(prepare_script)
            package(distro, "first")
            # A failed initial install has an explicit repair, with no install on Start.
            installer = (recipes / "install.sh").read_text()
            (recipes / "install.sh").write_text("echo 'Fixture create install failure'\nexit 7\n")
            recovery = {"name": "Install recovery", "distribution": distro, "packages": []}
            if distro == "arch":
                recovery["docker_args"] = docker_args
            probe = api("/sessions", "POST", recovery)["id"]
            created.append(probe)
            wait(lambda: state(probe)["status"] == "failed")
            api(f"/sessions/{probe}/stop", "POST")
            restart_manager()
            wait(lambda: state(probe)["repair_available"])
            rejected_action(probe, "start", 409)
            (tools / "fail-cp").touch()
            rejected_action(probe, "upgrade", 500)
            (tools / "fail-cp").unlink()
            (recipes / "install.sh").write_text(installer)
            api(f"/sessions/{probe}/upgrade", "POST")
            wait(lambda: state(probe)["status"] == "stopped", timeout=90)
            assert not state(probe)["repair_available"]
            api(f"/sessions/{probe}/start", "POST")
            wait(lambda: state(probe)["status"] == "running")
            check_docker_args(probe, docker_args if distro == "arch" else [])
            probe_name = "innkeeper-" + probe
            if distro == "arch":
                run("docker", "exec", probe_name, "ln", "-s", "/tmp/unreadable", "/var/lib/pacman/local/elsewhere-invalid")
                restart_manager()
                wait(lambda: state(probe)["installed_version"] is None)
                assert not state(probe)["repair_available"]
                rejected_action(probe, "upgrade", 500)
                run("docker", "exec", probe_name, "rm", "/var/lib/pacman/local/elsewhere-invalid")
            else:
                # A pending journal can supersede status; never authorize from stale metadata.
                run("docker", "exec", probe_name, "sh", "-c", "printf 'Package: elsewhere\\nStatus: install ok unpacked\\nVersion: 99.0.0-1\\nArchitecture: amd64\\nDescription: Pending fixture\\n' > /var/lib/dpkg/updates/0000")
                restart_manager()
                wait(lambda: state(probe)["version_error"] is not None)
                assert "journal is pending" in state(probe)["version_error"]
                assert state(probe)["installed_version"] is None and not state(probe)["repair_available"]
                rejected_action(probe, "upgrade", 500)
                run("docker", "exec", probe_name, "rm", "/var/lib/dpkg/updates/0000")
                # Settled dpkg metadata can describe an incomplete installation.
                archive = package(distro, "first")
                run("docker", "cp", str(archive), probe_name + ":/tmp/fixture.deb")
                run("docker", "exec", probe_name, "dpkg", "--unpack", "/tmp/fixture.deb")
                api(f"/sessions/{probe}/stop", "POST")
                restart_manager()
                wait(lambda: state(probe)["repair_available"])
                assert state(probe)["installed_version"] == version + "-1"
                package(distro, "repaired", depends="ed")
                api(f"/sessions/{probe}/upgrade", "POST")
                wait(lambda: state(probe)["status"] == "stopped", timeout=90)
                assert not state(probe)["repair_available"]
                api(f"/sessions/{probe}/start", "POST")
                wait(lambda: state(probe)["status"] == "running")
                assert run("docker", "exec", probe_name, "cat", "/home/elsewhere/launches").splitlines()[-1] == "repaired"
                assert run("docker", "exec", probe_name, "dpkg-query", "-W", "-f=${db:Status-Status}", "ed") == "installed"
                archive = package(distro, "newer", "99.0.0")
                run("docker", "cp", str(archive), probe_name + ":/tmp/fixture.deb")
                run("docker", "exec", probe_name, "dpkg", "--unpack", "/tmp/fixture.deb")
                api(f"/sessions/{probe}/stop", "POST")
                restart_manager()
                wait(lambda: state(probe)["version_status"] == "newer")
                assert state(probe)["repair_available"]
                assert "installation is incomplete" in rejected_action(probe, "start", 409)
                package(distro, "first")
                api(f"/sessions/{probe}/upgrade", "POST")
                wait(lambda: state(probe)["status"] == "stopped", timeout=90)
                assert state(probe)["installed_version"] == version + "-1"
                assert not state(probe)["repair_available"]
            api(f"/sessions/{probe}", "DELETE")
            created.remove(probe)
            print(f"{distro}: cancelled/failed creation and explicit initial-install repair passed", flush=True)
            sid = api("/sessions", "POST", {"name": "Refresh " + distro,
                      "distribution": distro, "packages": [], "docker_args": docker_args})["id"]
            created.append(sid)
            name = "innkeeper-" + sid
            wait(lambda: state(sid)["status"] == "running")
            check_docker_args(sid, docker_args)
            assert state(sid)["installed_version"] == version + "-1"
            assert state(sid)["version_status"] == "current"
            identity = run("docker", "inspect", name, "--format", "{{.Id}}")
            def install_fixture(label, installed):
                archive = package(distro, label, installed)
                destination = "/tmp/fixture." + ("pkg.tar.zst" if distro == "arch" else "deb")
                run("docker", "cp", str(archive), name + ":" + destination)
                if distro == "arch":
                    run("docker", "exec", name, "pacman", "-U", "--noconfirm", destination)
                else:
                    run("docker", "exec", name, "dpkg", "-i", destination)
                return archive
            cached = install_fixture("old", "0.0.1")
            # A newer cache entry cannot change the package during Start.
            package(distro, "upgraded")
            run("docker", "exec", name, "sh", "-c", "printf 'exit 99\\n' > /opt/innkeeper/entrypoint.sh")
            api(f"/sessions/{sid}/stop", "POST")
            # Reload forces immediate metadata detection on the stopped container.
            restart_manager()
            wait(lambda: state(sid)["version_status"] == "older")
            check_docker_args(sid, docker_args)
            assert state(sid)["installed_version"] == "0.0.1-1"
            (tools / "no-base").touch()
            api(f"/sessions/{sid}/start", "POST")
            wait(lambda: state(sid)["status"] == "running")
            assert run("docker", "exec", name, "cat", "/home/elsewhere/launches").splitlines() == ["first", "old"]
            check_docker_args(sid, docker_args)
            api(f"/sessions/{sid}/stop", "POST")
            cached.unlink()
            (recipes / "prepare.sh").write_text("echo 'Fixture download failure'\nexit 1\n")
            api(f"/sessions/{sid}/upgrade", "POST")
            wait(lambda: state(sid)["status"] == "failed")
            api(f"/sessions/{sid}/stop", "POST")
            # Start has no download dependency, even with a failed upgrade and empty cache.
            api(f"/sessions/{sid}/start", "POST")
            wait(lambda: state(sid)["status"] == "running")
            assert launched(sid, 3)
            api(f"/sessions/{sid}/stop", "POST")
            (recipes / "prepare.sh").write_text("echo 'Fixture waiting'\nexec sleep 60\n")
            api(f"/sessions/{sid}/upgrade", "POST")
            wait(lambda: "Fixture waiting" in api(f"/sessions/{sid}/logs")["text"])
            api(f"/sessions/{sid}/stop", "POST")
            restart_manager()
            wait(lambda: state(sid)["version_status"] == "older")
            assert state(sid)["status"] == "stopped"
            # An upgrade exits without launching or applying pending desktop settings.
            pending_profile = {"name":"Upgrade test", "screen_size":{"width":1280,"height":720}, "kiosk":False, "software_encoding":True, "startup_command":"", "packages":[], "docker_args":docker_args, "gpu_access":False, "gpu_id":None}
            api(f"/sessions/{sid}/settings", "PUT", pending_profile)
            package(distro, "upgraded")
            api(f"/sessions/{sid}/upgrade", "POST")
            wait(lambda: state(sid)["status"] == "stopped")
            assert state(sid)["installed_version"] == version + "-1"
            assert state(sid)["settings_pending"]
            check_docker_args(sid, docker_args)
            assert run("docker", "inspect", name, "--format", "{{.State.Running}}") == "false"
            api(f"/sessions/{sid}/start", "POST")
            wait(lambda: state(sid)["status"] == "running")
            assert run("docker", "exec", name, "cat", "/home/elsewhere/launches").splitlines() == ["first", "old", "old", "upgraded"]
            # Failed maintenance is durable and Start never retries it.
            install_fixture("old-again", "0.0.1")
            package(distro, "upgraded")
            installer = (recipes / "install.sh").read_text()
            (recipes / "install.sh").write_text("echo 'Fixture install failure'\nexit 7\n")
            api(f"/sessions/{sid}/upgrade", "POST")
            wait(lambda: state(sid)["status"] == "failed")
            assert run("docker", "inspect", name, "--format", "{{.State.Running}}") == "false"
            restart_manager()
            time.sleep(4)
            assert state(sid)["status"] == "failed"
            api(f"/sessions/{sid}/stop", "POST")
            time.sleep(4)
            assert state(sid)["status"] == "stopped" and state(sid)["error"] is None
            api(f"/sessions/{sid}/start", "POST")
            wait(lambda: state(sid)["status"] == "running")
            assert run("docker", "exec", name, "cat", "/home/elsewhere/launches").splitlines()[-1] == "old-again"
            # Restart during a running desktop's upgrade download reconnects without a relaunch.
            cached.unlink()
            (recipes / "prepare.sh").write_text("echo 'Fixture interrupted download'\nexec sleep 60\n")
            api(f"/sessions/{sid}/upgrade", "POST")
            wait(lambda: "Fixture interrupted download" in api(f"/sessions/{sid}/logs")["text"])
            restart_manager()
            wait(lambda: state(sid)["status"] == "running")
            assert account.connect(sid)
            assert run("docker", "exec", name, "cat", "/home/elsewhere/launches").splitlines()[-1] == "old-again"
            package(distro, "upgraded")
            # A slow maintenance operation stays monitored across restart and late success.
            (recipes / "install.sh").write_text("echo 'Fixture install waiting'\nwhile [ ! -f /tmp/finish-upgrade ]; do sleep 1; done\n" + installer)
            api(f"/sessions/{sid}/upgrade", "POST")
            wait(lambda: "Fixture install waiting" in api(f"/sessions/{sid}/logs")["text"])
            manager.send_signal(signal.SIGINT)
            manager.wait(timeout=10)
            with database(data) as db:
                db.execute("UPDATE sessions SET upgrade_started_ms = 1 WHERE id = ?", [sid])
            manager = subprocess.Popen(["elsewhere-innkeeper"], env=env, stdout=log, stderr=log)
            def warned():
                try: return "longer than 30 minutes" in (state(sid)["error"] or "")
                except OSError: return False
            wait(warned)
            time.sleep(4)
            assert state(sid)["status"] == "upgrading"
            assert run("docker", "inspect", name, "--format", "{{.State.Running}}") == "true"
            run("docker", "exec", name, "touch", "/tmp/finish-upgrade")
            wait(lambda: state(sid)["status"] == "stopped", timeout=90)
            assert state(sid)["error"] is None
            assert state(sid)["installed_version"] == version + "-1"
            (recipes / "install.sh").write_text(installer)
            api(f"/sessions/{sid}/start", "POST")
            wait(lambda: state(sid)["status"] == "running")
            # Start keeps the installed version; explicit installation uses the preferred package.
            install_fixture("newer", "99.0.0")
            api(f"/sessions/{sid}/stop", "POST")
            restart_manager()
            wait(lambda: state(sid)["version_status"] == "newer")
            package(distro, "upgraded")
            api(f"/sessions/{sid}/start", "POST")
            wait(lambda: state(sid)["status"] == "running")
            assert state(sid)["version_status"] == "newer"
            for label in ("downgraded", "reinstalled"):
                package(distro, label)
                if distro == "debian" and label == "reinstalled":
                    # Reinstallation replaces the payload even if the status query fails.
                    run("docker", "exec", name, "sh", "-c", "printf '#!/bin/sh\\nexit 1\\n' > /usr/local/bin/dpkg-query; chmod +x /usr/local/bin/dpkg-query")
                api(f"/sessions/{sid}/upgrade", "POST")
                wait(lambda: state(sid)["status"] == "stopped", timeout=90)
                assert state(sid)["installed_version"] == version + "-1"
                assert state(sid)["error"] is None
                assert run("docker", "inspect", name, "--format", "{{.Id}}") == identity
                payload = work / "installed-elsewhere"
                run("docker", "cp", name + ":/usr/bin/elsewhere", str(payload))
                assert label in payload.read_text()
                api(f"/sessions/{sid}/start", "POST")
                wait(lambda: state(sid)["status"] == "running")
                assert run("docker", "exec", name, "cat", "/home/elsewhere/launches").splitlines()[-1] == label
                if distro == "debian" and label == "reinstalled":
                    run("docker", "exec", name, "rm", "/usr/local/bin/dpkg-query")
            assert not (tools / "image-attempt").exists()
            (tools / "no-base").unlink()
            api(f"/sessions/{sid}/settings", "PUT", dict(pending_profile, screen_size=None))
            api(f"/sessions/{sid}/relaunch", "POST")
            wait(lambda: state(sid)["status"] == "running")
            # Connect persists credentials and reuses the same active token.
            assert account.connect(sid) == account.connect(sid)
            baseline = len(run("docker", "exec", name, "cat", "/home/elsewhere/launches").splitlines())
            run("docker", "exec", name, "sh", "-c", "echo retained > /root/settings-sentinel")
            restart_manager()
            wait(lambda: state(sid)["status"] == "running")
            assert not state(sid)["settings_pending"]
            original = {k: state(sid)[k] for k in ("name", "screen_size", "kiosk", "startup_command", "software_encoding", "packages", "docker_args", "gpu_access", "gpu_id")}
            def save(settings):
                return api(f"/sessions/{sid}/settings", "PUT", settings)
            def pending():
                return state(sid)["settings_pending"]
            # Saving and renaming never restart the desktop; reverting clears pending.
            renamed = dict(original, name="Renamed")
            assert not save(renamed)["settings_pending"]
            command = "printf '%s' \"$(touch /tmp/unexpected)\";\n echo 'quoted'"
            edited = dict(renamed, screen_size={"width": 1280, "height": 720}, kiosk=True,
                          startup_command=command)
            assert save(edited)["settings_pending"]
            assert launched(sid, baseline)
            assert not save(renamed)["settings_pending"]
            save(edited)
            for bad in (dict(edited, screen_size={"width": 3, "height": 720}),
                        dict(edited, startup_command="\0"), dict(edited, name=" ")):
                rejected(f"/sessions/{sid}/settings", "PUT", bad, 400)
            missing = dict(edited)
            del missing["screen_size"]
            rejected(f"/sessions/{sid}/settings", "PUT", missing, 400)
            rejected(f"/sessions/{sid}/settings", "PUT", dict(edited, packages=["--bad"]), 400)
            rejected(f"/sessions/{sid}/settings", "PUT", dict(edited, docker_args=["--privileged=true"]), 400)
            # A failed write cannot publish edits in memory.
            reject_updates(data, sid, True)
            rejected(f"/sessions/{sid}/settings", "PUT", renamed, 500)
            assert pending() and state(sid)["kiosk"]
            reject_updates(data, sid, False)
            manager.send_signal(signal.SIGINT)
            manager.wait(timeout=10)
            manager = subprocess.Popen(["elsewhere-innkeeper"], env=env, stdout=log, stderr=log)
            def online():
                try:
                    return pending()
                except OSError:
                    return False
            wait(online)
            # Stop failure preserves the current desktop and pending settings.
            (tools / "fail-stop").touch()
            rejected(f"/sessions/{sid}/relaunch", "POST", None, 500)
            (tools / "fail-stop").unlink()
            assert launched(sid, baseline) and pending()
            # The operation lock admits only one of simultaneous relaunch requests.
            def restart():
                try:
                    api(f"/sessions/{sid}/relaunch", "POST")
                    return 202
                except urllib.error.HTTPError as e:
                    return e.code
            (work / "not-ready").touch()
            with concurrent.futures.ThreadPoolExecutor(2) as pool:
                assert sorted(pool.map(lambda _: restart(), range(2))) == [202, 409]
            wait(lambda: launched(sid, baseline + 1))
            assert pending() and state(sid)["status"] == "preparing"
            rejected(f"/sessions/{sid}/settings", "PUT", renamed, 409)
            (work / "not-ready").unlink()
            wait(lambda: state(sid)["status"] == "running" and not pending())
            assert launched(sid, baseline + 1)
            args = subprocess.check_output(["docker", "exec", name, "cat", "/home/elsewhere/launch-args"]).decode().split("\0")[:-1]
            assert args == ["--no-tls", "--listen", "0.0.0.0:19443", "--url-prefix", "/e/" + sid, "--rtc-port", str(state(sid)["port"]), "--elements", "--render-node", "none", "--rtc-addr", "127.0.0.1",
                            "--screen-size", "1280x720", "--software-encoding", "--kiosk", "--exec", command], args
            run("docker", "exec", name, "test", "!", "-e", "/tmp/unexpected")
            assert run("docker", "inspect", name, "--format", "{{.Id}}") == identity
            check_docker_args(sid, docker_args)
            assert run("docker", "exec", name, "cat", "/root/settings-sentinel") == "retained"
            assert stored_settings(data, sid, "launching") is None, stored_settings(data, sid, "launching")
            assert stored_settings(data, sid, "applied") == {k: edited[k] for k in ("screen_size", "kiosk", "startup_command", "software_encoding")}
            # Copy and Docker start failures retain edits for a later start.
            for failure in ("cp", "start"):
                save(renamed)
                (tools / ("fail-" + failure)).touch()
                api(f"/sessions/{sid}/relaunch", "POST")
                wait(lambda: state(sid)["status"] == "failed")
                error = state(sid)["error"]
                time.sleep(4)
                assert pending() and state(sid)["status"] == "failed" and state(sid)["error"] == error
                (tools / ("fail-" + failure)).unlink()
                api(f"/sessions/{sid}/stop", "POST")
                assert save(renamed)["settings_pending"]
                api(f"/sessions/{sid}/start", "POST")
                wait(lambda: state(sid)["status"] == "running" and not pending())
                args = subprocess.check_output(["docker", "exec", name, "cat", "/home/elsewhere/launch-args"]).decode().split("\0")[:-1]
                assert args == ["--no-tls", "--listen", "0.0.0.0:19443", "--url-prefix", "/e/" + sid, "--rtc-port", str(state(sid)["port"]), "--elements", "--render-node", "none", "--rtc-addr", "127.0.0.1", "--software-encoding"]
                save(edited)
                api(f"/sessions/{sid}/relaunch", "POST")
                wait(lambda: state(sid)["status"] == "running" and not pending())
            # A persistence failure after stopping leaves settings available for retry.
            save(renamed)
            reject_updates(data, sid, True)
            rejected(f"/sessions/{sid}/relaunch", "POST", None, 500)
            assert run("docker", "inspect", name, "--format", "{{.State.Running}}") == "false"
            assert pending()
            reject_updates(data, sid, False)
            wait(lambda: state(sid)["status"] == "stopped")
            api(f"/sessions/{sid}/start", "POST")
            wait(lambda: state(sid)["status"] == "running" and not pending())
            check_docker_args(sid, docker_args)
            # Replacement preserves the complete installation, including files outside the home volume.
            old_id = run("docker", "inspect", name, "--format", "{{.Id}}")
            old_version = state(sid)["installed_version"]
            old_hostname = run("docker", "inspect", name, "--format", "{{.Config.Hostname}}")
            shutil.copyfile(source / "packages.sh", recipes / "packages.sh")
            run("docker", "exec", name, "sh", "-c", "echo home > /home/elsewhere/replacement-sentinel")
            with database(data) as db:
                old_tokens = [tuple(row) for row in db.execute("SELECT token_id,secret FROM instance_tokens WHERE session_id=? ORDER BY token_id", [sid])]
            # The rig has no /dev/dri mount: missing hardware must fail before stopping the desktop.
            with database(data) as db:
                db.execute("UPDATE sessions SET gpu_access=1,gpu=? WHERE id=?", [json.dumps(dict(id="missing",driver="i915",node="/dev/dri/renderD999",major=226,minor=999)),sid])
            assert "compatible GPU" in rejected_action(sid, "relaunch", 400)
            assert run("docker", "inspect", name, "--format", "{{.State.Running}}") == "true"
            assert run("docker", "inspect", name, "--format", "{{.Id}}") == old_id
            with database(data) as db:
                db.execute("UPDATE sessions SET gpu_access=0,gpu=NULL WHERE id=?", [sid])
            replacement_profile = dict(renamed, docker_args=["--cap-drop=NET_RAW"], packages=["bash", "tree"])
            # A cancelled snapshot must never replace newer writes from the same source.
            save(replacement_profile)
            (tools / "fail-rm").touch()
            api(f"/sessions/{sid}/relaunch", "POST")
            wait(lambda: state(sid)["status"] == "failed", 120)
            with database(data) as db:
                cancelled = json.loads(db.execute("SELECT replacement FROM sessions WHERE id=?", [sid]).fetchone()[0])
            assert cancelled["image"]
            (tools / "fail-rm").unlink()
            api(f"/sessions/{sid}/stop", "POST")
            save(renamed)
            api(f"/sessions/{sid}/start", "POST")
            wait(lambda: state(sid)["status"] == "running")
            run("docker", "exec", name, "sh", "-c", "echo newer > /root/after-cancelled-snapshot")
            assert save(replacement_profile)["settings_pending"]
            assert not save(renamed)["settings_pending"]
            save(replacement_profile)
            (tools / "fail-commit").touch()
            api(f"/sessions/{sid}/relaunch", "POST")
            wait(lambda: state(sid)["status"] == "failed")
            assert run("docker", "inspect", name, "--format", "{{.Id}}") == old_id
            api(f"/sessions/{sid}/stop", "POST")
            save(renamed)
            api(f"/sessions/{sid}/start", "POST")
            wait(lambda: state(sid)["status"] == "running")
            assert run("docker", "inspect", name, "--format", "{{.Id}}") == old_id
            save(replacement_profile)
            api(f"/sessions/{sid}/relaunch", "POST")
            wait(lambda: state(sid)["status"] == "failed")
            (tools / "fail-commit").unlink()
            api(f"/sessions/{sid}/stop", "POST")
            (tools / "fail-replacement-create").touch()
            snapshot_started = time.monotonic()
            api(f"/sessions/{sid}/upgrade", "POST")
            wait(lambda: state(sid)["status"] == "failed", 120)
            with database(data) as db:
                replacement = json.loads(db.execute("SELECT replacement FROM sessions WHERE id=?", [sid]).fetchone()[0])
            image_id = replacement["image"]
            assert image_id and run("docker", "image", "inspect", image_id, "--format", "{{json .RepoTags}}") != "[]"
            assert not run("docker", "ps", "-aq", "--filter", "name=^/" + name + "$")
            restart_manager()
            api(f"/sessions/{sid}/stop", "POST")
            (tools / "fail-replacement-create").unlink()
            save(dict(replacement_profile, docker_args=["--cap-add=NOT_A_CAPABILITY"]))
            api(f"/sessions/{sid}/start", "POST")
            wait(lambda: state(sid)["status"] == "failed")
            api(f"/sessions/{sid}/stop", "POST")
            save(replacement_profile)
            (tools / "fail-cp").touch()
            api(f"/sessions/{sid}/start", "POST")
            wait(lambda: state(sid)["status"] == "failed")
            replacement_id = run("docker", "inspect", name, "--format", "{{.Id}}")
            assert replacement_id != old_id
            (tools / "fail-cp").unlink()
            api(f"/sessions/{sid}/stop", "POST")
            api(f"/sessions/{sid}/start", "POST")
            wait(lambda: state(sid)["status"] == "running" and not pending(), 180)
            assert run("docker", "inspect", name, "--format", "{{.Id}}") == replacement_id
            assert run("docker", "inspect", name, "--format", "{{.Image}}") == image_id
            snapshot = json.loads(run("docker", "image", "inspect", image_id))[0]
            assert image_id != cancelled["image"]
            assert snapshot["RepoTags"] == [f"innkeeper-snapshot-{sid}:{replacement['snapshot']}"]
            assert snapshot["Config"]["Labels"]["io.innkeeper.snapshot-id"] == replacement["snapshot"]
            assert run("docker", "exec", name, "cat", "/root/after-cancelled-snapshot") == "newer"
            assert snapshot["Config"]["Labels"]["io.innkeeper.snapshot"] == "true"
            assert snapshot["Config"]["Labels"]["io.innkeeper.session"] == sid
            assert snapshot["Config"]["Labels"]["io.innkeeper.snapshot-source"] == old_id
            assert run("docker", "exec", name, "cat", "/root/settings-sentinel") == "retained"
            assert run("docker", "exec", name, "cat", "/home/elsewhere/replacement-sentinel") == "home"
            assert run("docker", "exec", name, "cat", "/opt/innkeeper/packages-installed") == "bash\ntree"
            run("docker", "exec", name, "tree", "--version")
            assert run("docker", "inspect", name, "--format", "{{.Config.Hostname}}") == old_hostname
            assert state(sid)["installed_version"] == old_version
            check_docker_args(sid, ["--cap-drop=NET_RAW"])
            with database(data) as db:
                assert [tuple(row) for row in db.execute("SELECT token_id,secret FROM instance_tokens WHERE session_id=? ORDER BY token_id", [sid])] == old_tokens
            # Removing a requested package retains installed programs and triggers just one replacement.
            save(dict(replacement_profile, packages=[]))
            api(f"/sessions/{sid}/relaunch", "POST")
            wait(lambda: state(sid)["status"] == "running" and not pending(), 120)
            assert run("docker", "inspect", name, "--format", "{{.Id}}") != replacement_id
            run("docker", "exec", name, "tree", "--version")
            assert run("docker", "exec", name, "cat", "/root/settings-sentinel") == "retained"
            print(f"{distro}: replacement, snapshot recovery, tagged images, settings and token preservation passed in {time.monotonic() - snapshot_started:.1f}s", flush=True)
            print(f"{distro}: Docker security options and capabilities survive restart, upgrade and relaunch", flush=True)
            print(f"{distro}: saved settings, resets, quoting, pending state, persistence, serialized relaunch and failure retry passed", flush=True)
            print(f"{distro}: explicit upgrade, downgrade, reinstall, stopped version detection, launch-only start, cancellation, managed token reuse and restart persistence passed", flush=True)
    except BaseException:
        print("Session states:", api("/sessions"), flush=True)
        with database(data) as db:
            print("Stored settings:", [dict(row) for row in db.execute("SELECT * FROM session_settings")], flush=True)
        for sid in created:
            subprocess.run(["docker", "logs", "--tail", "80", "innkeeper-" + sid])
        raise
    finally:
        for sid in created:
            try:
                api(f"/sessions/{sid}", "DELETE")
            except Exception:
                subprocess.run(["docker", "rm", "-f", "innkeeper-" + sid], capture_output=True)
                subprocess.run(["docker", "volume", "rm", "innkeeper-" + sid + "-data"], capture_output=True)
        for sid in created:
            images = run("docker", "image", "ls", "-q", "--filter", "label=io.innkeeper.snapshot=true", "--filter", "label=io.innkeeper.session=" + sid).splitlines()
            for image in dict.fromkeys(images):
                subprocess.run(["docker", "image", "rm", image], capture_output=True)
        manager.terminate()
        manager.wait(timeout=10)
        log.seek(0)
        print(log.read())
