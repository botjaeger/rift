"""Settings collection editors through real headless input and persisted TOML.

Called by ui-smoke.py; every run uses its own config, HOME and data directory.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import tomllib

# clearing a text field: ⌘⌫ on macOS, Ctrl+Backspace elsewhere (input::primary)
P = "cmd" if sys.platform == "darwin" else "ctrl"


def main():
    started = time.monotonic()
    binary = Path(sys.argv[1]).resolve()
    root = Path(tempfile.mkdtemp(prefix="rift-settings-editors-", dir="/tmp"))
    print(f"Settings editor evidence: {root}", flush=True)
    for name in ("home", "cfg/rift", "data", "project/sub"):
        (root / name).mkdir(parents=True)
    path = root / "cfg/rift/config.toml"
    base = ('# keep my comment\nshell="/bin/bash"\nrestore_session=false\n'
            'durable_sessions=false\ndiscover_shells=false\nsync_os_theme=false\n'
            'mask_secrets=true\nsave_transcripts=true\n'
            f'working_directory={json.dumps(str(root / "project"))}\n')
    path.write_text(base)
    env = {k: v for k, v in os.environ.items() if not k.startswith("RIFT_")}
    for key in ("ZDOTDIR", "BASH_ENV", "ENV", "SSH_AUTH_SOCK"):
        env.pop(key, None)
    env.update(HOME=str(root / "home"), XDG_CONFIG_HOME=str(root / "cfg"),
               XDG_DATA_HOME=str(root / "data"), RIFT_NO_NOTIFY="1")

    def encoded(value):
        return value.replace("\\", "\\\\").replace("\n", "\\n").replace("\t", "\\t")

    def click(label):
        return [f"a11y click {label}", "wait 80"]

    def field(label, value):
        return [*click(label), f"key {P}+backspace", f"paste {encoded(value)}", "key enter", "wait 80"]

    def capture(name):
        return ["wait 200", f"dump {root}/{name}.txt", f"shot {root}/{name}.png",
                f"sh cp {path} {root}/{name}.toml"]

    def run(name, page, steps):
        script = root / f"{name}.rift"
        script.write_text("\n".join(["wait_prompt", "resize 1300 1500", "key cmd+,",
                                      "wait 200", *(click(page) if page != "General" else []), *steps, "quit"]) + "\n")
        with (root / f"{name}.log").open("w") as log:
            result = subprocess.run([str(binary), "--script", str(script), "--headless"],
                                    env=env, stdout=log, stderr=log, timeout=100)
        logs = (root / f"{name}.log").read_text()
        assert result.returncode == 0, logs[-6000:]
        assert "no node labelled" not in logs, logs[-8000:]
        assert "wait_prompt timed out" not in logs, logs[-8000:]

    def data(name):
        return tomllib.loads((root / f"{name}.toml").read_text())

    def dump(name):
        return (root / f"{name}.txt").read_text()

    def geometry(name, width):
        line=next(l for l in dump(name).splitlines() if l.startswith('settings_controls='))
        controls=json.loads(line.split('=',1)[1])
        for rect, label in controls:
            x,y,w,h=rect
            assert x>=0 and x+w<=width+1, (name,label,rect)
        for i,(a,an) in enumerate(controls):
            for b,bn in controls[i+1:]:
                overlap=min(a[0]+a[2],b[0]+b[2])-max(a[0],b[0])>3 and min(a[1]+a[3],b[1]+b[3])-max(a[1],b[1])>3
                contains=lambda r,s:r[0]<=s[0] and r[1]<=s[1] and r[0]+r[2]>=s[0]+s[2] and r[1]+r[3]>=s[1]+s[3]
                assert not overlap or contains(a,b) or contains(b,a), (name,an,bn,a,b)

    run("ssh", "Profiles", [*click("Add SSH profile"),
        *field("SSH name: New SSH profile", "fixture"),
        *click("Save SSH profile"), *capture("ssh-invalid"),
        *field("SSH host: none", "fixture-alias"),
        *field("SSH arguments: none", "-p 2222 -o 'RemoteCommand=echo fixture'"),
        *capture("ssh-draft"), *click("Save SSH profile"), *capture("ssh-saved"),
        *click("Cancel SSH profile edit"), *click("Add SSH profile"),
        *field("SSH name: New SSH profile", "fixture"), *field("SSH host: none", "other"),
        *click("Save SSH profile"), *capture("ssh-duplicate"), *click("Cancel SSH profile edit")])
    assert "ssh_profiles" not in data("ssh-draft")
    assert "Enter a host" in dump("ssh-invalid")
    assert data("ssh-saved")["ssh_profiles"] == [{"name": "fixture", "host": "fixture-alias",
        "args": ["-p", "2222", "-o", "RemoteCommand=echo fixture"]}]
    assert "already used" in dump("ssh-duplicate")
    assert data("ssh-duplicate") == data("ssh-saved")

    sent = f"printf '  first\\nsecond  \\n' > {root}/action-result\n"
    run("actions", "Actions", [*click("Add action"),
        *field("Action name: New action", "Write fixture"),
        *field("Send text", sent), *click("Record action shortcut"), "key ctrl+alt+f9",
        *capture("action-draft"), *click("Save action"), *capture("action-saved"),
        *click("Cancel action edit"), "key escape", "wait 200", "key ctrl+alt+f9",
        "wait 400", "wait_prompt", f"dump {root}/action-ran.txt"])
    assert "actions" not in data("action-draft")
    assert data("action-saved")["actions"][0]["steps"] == [{"send": sent}]
    assert (root / "action-result").read_text() == "  first\nsecond  \n"

    # Durable sessions remain off: persistence here writes only this fixture's
    # transcript, without starting a session service.
    env['RIFT_PERSIST_SESSION'] = '1'
    run("secret", "Terminal", [*click("Add secret pattern"),
        *field("Pattern name: New secret pattern", "Fixture marker"),
        *field("Regular expression: none", "["), *click("Save secret pattern"),
        *capture("secret-invalid"), *field("Regular expression: [", "fixture=(MARKER[0-9]+)"),
        *field("Sample text", "fixture=MARKER123 harmless"), *capture("secret-preview"),
        # the preview follows the typing (before Enter) and shows this pattern
        # alone: a built-in would mask the token
        *click("Sample text"), f"key {P}+backspace",
        f"paste {encoded('token=abcdef123456 fixture=MARKER9')}", *capture("secret-live"),
        "key enter", "wait 80",
        *click("Save secret pattern"), *capture("secret-saved"),
        *click("Cancel secret pattern edit"), "key escape", "wait 200",
        "type printf 'fixture=MARKER123 harmless\\\\n'\\n", "wait 400", "wait_prompt",
        "key cmd+c", "wait 100", f"dump {root}/secret-output.txt",
        f"sh ({binary} ctl blocks --pane 1 > {root}/secret-control.json; touch {root}/control-done) &",
        f"wait_file {root}/control-done 5000",
        "rclick 800 400", "wait 200", *click("Export buffer…"),
        f"type {root}/secret-export.txt", "key enter", "wait 200",
        f"sh cp -R {root}/data/rift/transcripts {root}/saved-transcripts"])
    env.pop('RIFT_PERSIST_SESSION')
    assert "secret_patterns" not in data("secret-invalid")
    assert "regex" in dump("secret-invalid") and "Not saved:" in dump("secret-invalid")
    assert "Preview: fixture=••••••••• harmless" in dump("secret-preview")
    assert "Preview: token=abcdef123456 fixture=•••••••" in dump("secret-live"), dump("secret-live")[-3000:]
    assert data("secret-saved")["secret_patterns"] == [{"name": "Fixture marker", "regex": "fixture=(MARKER[0-9]+)"}]
    assert "MARKER123" not in (root / "secret-saved.toml").read_text()
    assert 'clipboard="fixture=••••••••• harmless\\n"' in dump("secret-output")
    for output in ('secret-control.json','secret-export.txt'):
        text = (root / output).read_text()
        assert 'MARKER123' not in text and '•••••••••' in text, (output,text)
    transcripts = [p.read_text() for p in (root/'saved-transcripts').glob('*.json')]
    assert transcripts and all('MARKER123' not in t for t in transcripts)
    assert any('•••••••••' in t for t in transcripts)

    run("colors", "Appearance", [*click("Add tab color rule"),
        *field("Rule directory: none", str(root / "project")),
        *field("Rule color: blue", "wrong"), *click("Save tab color rule"), *capture("color-invalid"),
        *field("Rule color: wrong", "#123456"), *click("Save tab color rule"), *capture("color-saved"),
        'key escape', *(['key tab','wait 80'] * 10), 'key right', *capture('color-slider-draft'),
        *click('Save tab color rule'), *capture('color-slider-saved'),
        *click('Pick rule color cyan'), *click('Save tab color rule'), *capture('color-named'),
        *field('Rule color: cyan','#123456'), *click('Save tab color rule'),
        *click("Cancel tab color rule edit"), "key escape", "wait 100", f"dump {root}/color-tab.txt"])
    assert "tab_color_rules" not in data("color-invalid")
    assert "Choose a named color" in dump("color-invalid")
    assert data("color-saved")["tab_color_rules"] == [{"dir": str(root / "project"), "color": "#123456"}]
    assert data('color-slider-draft') == data('color-saved')
    assert 'settings_focus=Some(SettingsSlider("collection.red"))' in dump('color-slider-draft')
    assert data('color-slider-saved')['tab_color_rules'][0]['color'] != '#123456'
    assert data('color-named')['tab_color_rules'][0]['color'] == 'cyan'
    assert '#123456' in dump("color-tab")
    assert path.read_text().startswith("# keep my comment\n")

    # Reopen persisted entries and delete through the confirmation controls.
    for page, kind, name, key in [("Profiles", "SSH profile", "fixture", "ssh_profiles"),
            ("Actions", "action", "Write fixture", "actions"),
            ("Terminal", "secret pattern", "Fixture marker", "secret_patterns"),
            ("Appearance", "tab color rule", str(root / "project"), "tab_color_rules")]:
        run(f"delete-{key}", page, [*click(f"Edit {kind} {name}"), *click(f"Delete {kind}"),
            *capture(f"confirm-{key}"), *click(f"Cancel {kind} edit"),
            *capture(f"cancel-delete-{key}"), *click(f"Delete {kind}"),
            *click(f"Confirm delete {kind}"), *capture(f"deleted-{key}")])
        assert data(f"confirm-{key}")[key]
        assert data(f"cancel-delete-{key}")[key]
        assert 'settings_collection=Some(' in dump(f"cancel-delete-{key}")
        assert not data(f"deleted-{key}").get(key)
    # Three-entry reorder and stale-draft rejection on every collection.
    cases = [
        ("Profiles", "SSH profile", "ssh_profiles", "SSH host: host-b", "new-host", "host",
         lambda n: f'name="{n}"\nhost="host-{n}"\n'),
        ("Actions", "action", "actions", "Action name: b", "renamed", "name",
         lambda n: f'name="{n}"\nsteps=[{{send="echo {n}"}}]\n'),
        ("Terminal", "secret pattern", "secret_patterns", "Regular expression: match-b", "new-match", "regex",
         lambda n: f'name="{n}"\nregex="match-{n}"\n'),
        ("Appearance", "tab color rule", "tab_color_rules", "Rule color: blue", "red", "color",
         lambda n: f'dir="{n}"\ncolor="blue"\n'),
    ]
    for page, kind, key, edit, value, changed, entry in cases:
        initial = base + ''.join(f'\n# entry {n}\n[[{key}]]\n{entry(n)}' for n in ('a','b','c'))
        path.write_text(initial)
        external = root / f"external-{key}.toml"
        external.write_text(base + ''.join(f'\n# entry {n}\n[[{key}]]\n{entry(n)}' for n in ('c','a','b')))
        run(f"stale-{key}", page, [*click(f"Edit {kind} b"), *field(edit, value),
            f"sh cp {external} {path}", "wait 1200", *click(f"Save {kind}"),
            *capture(f"stale-{key}"), *click(f"Cancel {kind} edit"),
            *click(f"Edit {kind} b"), *click(f"Move {kind} up"), *capture(f"moved-{key}")])
        assert "changed elsewhere" in dump(f"stale-{key}")
        identity = "dir" if key == "tab_color_rules" else "name"
        assert [e[identity] for e in data(f"stale-{key}")[key]] == ['c','a','b']
        assert [e[identity] for e in data(f"moved-{key}")[key]] == ['c','b','a']
        assert data(f"stale-{key}")[key][-1][changed] != value
        comments = (root / f"moved-{key}.toml").read_text()
        assert comments.index('# entry c') < comments.index('# entry b') < comments.index('# entry a')

    # Cycles, referenced deletion, and renaming callers use the same validator
    # as file loading. Saving the caller still must not execute it.
    path.write_text(base + '\n[[actions]]\nname="callee"\nsteps=[{send="echo idle"}]\n'
                    '\n[[actions]]\nname="caller"\nsteps=[{action="callee"}]\n')
    run("references", "Actions", [*click("Edit action callee"),
        *click("Delete action"), *click("Confirm delete action"), *capture("referenced-delete"),
        *click("Cancel action edit"), *click("Edit action callee"),
        *field("Action name: callee", "renamed"), *click("Save action"), *capture("renamed"),
        *click("Cancel action edit"), *click("Edit action caller"),
        *field("Run action: renamed", "caller"), *click("Save action"), *capture("cycle")])
    assert len(data("referenced-delete")["actions"]) == 2
    assert data("renamed")["actions"][1]["steps"] == [{"action": "renamed"}]
    assert data("cycle") == data("renamed")
    assert 'Not saved:' in dump("cycle")

    # A second window deleting a preceding entry cannot redirect a stale draft.
    path.write_text(base + ''.join(f'\n[[ssh_profiles]]\nname="{n}"\nhost="host-{n}"\n' for n in 'abc'))
    run('windows', 'Profiles', [*click('Edit SSH profile b'),
        *field('SSH host: host-b','draft-host'), 'new_window', 'wait_prompt',
        'resize 1300 1500', 'key cmd+,', 'wait 200', *click('Profiles'),
        *click('Edit SSH profile a'), *click('Delete SSH profile'),
        *click('Confirm delete SSH profile'), 'window 0', 'wait 200',
        *click('Save SSH profile'), *capture('windows-stale')])
    assert [p['name'] for p in data('windows-stale')['ssh_profiles']] == ['b','c']
    assert data('windows-stale')['ssh_profiles'][0]['host'] == 'host-b'
    assert 'changed elsewhere' in dump('windows-stale')

    path.write_text(base + '\n[[ssh_profiles]]\nname="visible-fixture"\nhost="fixture-alias"\n')
    run('palette-windows', 'Profiles', [*click('Edit SSH profile visible-fixture'),
        'new_window','wait_prompt','key cmd+p','type SSH: visible-fixture','wait 200',
        f'dump {root}/palette-before.txt','window 0',*click('Delete SSH profile'),
        *click('Confirm delete SSH profile'),'window 1','wait 200',
        f'dump {root}/palette-after.txt'])
    assert 'results=1 ' in dump('palette-before')
    assert 'results=0 ' in dump('palette-after')

    # Failed saves retain drafts; fixing permissions permits a retry. A linked
    # config remains linked, and unrelated disk edits survive a collection save.
    path.write_text(base)
    run('storage', 'Profiles', [*click('Add SSH profile'),
        *field('SSH host: none','fixture-alias'), f'sh chmod 500 {path.parent}',
        *click('Save SSH profile'), *capture('storage-error'),
        f'sh chmod 700 {path.parent}', *click('Save SSH profile'), *capture('storage-retry')])
    assert not data('storage-error').get('ssh_profiles')
    assert 'Could not save:' in dump('storage-error')
    assert data('storage-retry')['ssh_profiles'][0]['host'] == 'fixture-alias'
    path.write_text(base)
    broken = root / 'broken.toml'
    broken.write_text(base + 'font_size=[\n')
    repaired = root / 'repaired.toml'
    repaired.write_text(base)
    run('invalid-file', 'Profiles', [*click('Add SSH profile'),
        *field('SSH host: none','retained-host'), f'sh cp {broken} {path}',
        'wait 1200', *click('Save SSH profile'), *capture('invalid-file'),
        f'sh cp {repaired} {path}','wait 1200',*click('Save SSH profile'),
        *capture('repaired-file')])
    assert (root/'invalid-file.toml').read_text() == broken.read_text()
    assert 'Fix config.toml before saving' in dump('invalid-file')
    assert data('repaired-file')['ssh_profiles'][0]['host'] == 'retained-host'
    target = root / 'linked.toml'
    target.write_text(base)
    path.unlink()
    path.symlink_to(target)
    external = root / 'unrelated.toml'
    external.write_text(base + 'font_size=19\n')
    run('linked', 'Profiles', [*click('Add SSH profile'),
        *field('SSH host: none','fixture-alias'), f'sh cp {external} {target}',
        *click('Save SSH profile'), *capture('linked-saved')])
    assert path.is_symlink()
    assert data('linked-saved')['font_size'] == 19
    assert data('linked-saved')['ssh_profiles'][0]['host'] == 'fixture-alias'
    path.unlink()
    path.write_text(base)

    # Shortcut conflict feedback, then reordering keeps shortcuts with names.
    path.write_text(base + ''.join(f'\n[[actions]]\nname="{n}"\nkey="ctrl+alt+f{j}"\n'
        f'steps=[{{send="echo {n} > {root}/shortcut-{n}\\n"}}]\n' for n,j in [('a',8),('b',9),('c',10)]))
    run('shortcuts', 'Actions', [*click('Edit action b'),
        *field('Action shortcut: ctrl+alt+f9','cmd+p'), *click('Save action'),
        *capture('shortcut-conflict'), *field('Action shortcut: cmd+p','ctrl+alt+f9'),
        *click('Move action up'), *capture('shortcut-reordered'), *click('Cancel action edit'),
        'key escape','key ctrl+alt+f9',f'wait_file {root}/shortcut-b 5000',
        'wait_prompt','key ctrl+alt+f8',f'wait_file {root}/shortcut-a 5000'])
    assert 'Shortcut is already used' in dump('shortcut-conflict')
    assert [a['name'] for a in data('shortcut-reordered')['actions']] == ['b','a','c']
    assert (root/'shortcut-b').read_text() == 'b\n'
    assert (root/'shortcut-a').read_text() == 'a\n'

    # Assemble and reorder all three action step types in the editor.
    layouts = root / "cfg/rift/launch_configs"
    layouts.mkdir()
    (layouts / "fixture.yaml").write_text(f'name: fixture\nwindows:\n  - tabs:\n      - title: fixture\n        layout:\n          cwd: {root / "project"}\n')
    path.write_text(base)
    command = f"echo ORDERED > {root}/ordered-result\n"
    run("steps", "Actions", [*click("Add action"),
        *field("Action name: New action", "Ordered"), *field("Send text", command),
        *click("Add action step"), *click("Step type: Run action"),
        *field("Run action: none", "new_window"), *click("Move action step up"),
        *click("Add action step"), *click("Step type: Open layout"),
        *field("Launch configuration: none", "missing"), *click("Save action"), *capture("missing-layout"),
        *field("Launch configuration: missing", "fixture"), *click("Move action step up"),
        *click("Save action"), *capture("steps-saved"),
        f"sh test ! -e {root}/ordered-result && touch {root}/save-did-not-run",
        *click("Cancel action edit"), "key escape", "key cmd+p", "type Action: Ordered",
        "wait 200", "key enter", f"wait_file {root}/ordered-result 10000", "wait 300",
        f"dump {root}/steps-ran.txt"])
    assert 'No launch configuration named' in dump("missing-layout")
    assert data("steps-saved")["actions"][0]["steps"] == [
        {"action": "new_window"}, {"open": "fixture"}, {"send": command}]
    assert (root / "save-did-not-run").exists()
    assert (root / "ordered-result").read_text() == "ORDERED\n"

    # Color consumers share precedence in both tab layouts, including a manual
    # override and a directory reached through a symlink.
    (root / 'home/alias').symlink_to(root / 'project/sub', target_is_directory=True)
    for layout in ('horizontal','vertical'):
        path.write_text(base + f'tab_layout="{layout}"\n'
            '\n[keybindings]\nset_tab_color="ctrl+alt+h"\n'
            + ''.join(f'\n[[tab_color_rules]]\ndir={json.dumps(str(root / directory))}\ncolor="{color}"\n'
                for directory,color in [('project','blue'),('project/sub','green'),('project/sub','red')]))
        run(f'color-{layout}', 'Appearance', [*click(f'Edit tab color rule {root / "project"}'),
            *field('Rule color: blue','#123456'), *click('Save tab color rule'),
            *click('Cancel tab color rule edit'), 'key escape', 'wait 200',
            f'dump {root}/color-{layout}-base.txt', 'type cd sub\\n', 'wait 300', 'wait_prompt',
            f'dump {root}/color-{layout}-sub.txt', 'key ctrl+alt+h','type cyan','key enter',
            f'type cd {root / "home/alias"}\\n', 'wait 300','wait_prompt',
            f'dump {root}/color-{layout}-manual.txt','key ctrl+alt+h','type no color','key enter',
            'wait 200',f'dump {root}/color-{layout}-none.txt', 'key cmd+t',
            'wait_prompt', f'dump {root}/color-{layout}-auto.txt'])
        assert 'tab_tints=[Some("#123456")]' in dump(f'color-{layout}-base')
        assert 'tab_tints=[Some("red")]' in dump(f'color-{layout}-sub')
        assert 'tab_tints=[Some("cyan")]' in dump(f'color-{layout}-manual')
        assert 'tab_tints=[Some("red")]' in dump(f'color-{layout}-none')
        assert 'tab_tints=[Some("red"), Some("#123456")]' in dump(f'color-{layout}-auto')

    # All editors stay discoverable through search, and render in light/dark
    # themes at ordinary and compact desktop sizes without changing settings.
    for theme in ("dark-default", "light-default"):
        path.write_text(base + f'theme="{theme}"\n')
        for page, kind, query in [("Profiles", "SSH profile", "SSH profiles"),
                ("Actions", "action", "Custom actions"),
                ("Terminal", "secret pattern", "Secret patterns"),
                ("Appearance", "tab color rule", "Tab color rules")]:
            tag=f"layout-{theme}-{page}"
            name_field = {'Profiles':'SSH name: New SSH profile', 'Actions':'Action name: New action',
                          'Terminal':'Pattern name: New secret pattern', 'Appearance':'Rule directory: none'}[page]
            run(tag, "General", [f"type {query}", "wait 200", *click(f"Add {kind}"),
                *field(name_field, 'long fixture name ' * 12),
                "resize 800 600", "wait 200", f"shot {root}/{tag}.png", f"dump {root}/{tag}.txt",
                "key tab", "key shift+tab", "wait 100", f"dump {root}/{tag}-focus.txt"])
            assert 'settings_collection=Some(' in dump(tag)
            assert 'settings_focus=Some(' in dump(tag + '-focus')
            geometry(tag,800)
    print(f"Settings editors: CRUD, validation, draft isolation, consumers, stale writes, ordering, references, steps and layouts passed ({time.monotonic() - started:.1f}s)", flush=True)


if __name__ == "__main__":
    main()
