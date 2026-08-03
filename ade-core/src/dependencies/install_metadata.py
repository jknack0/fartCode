#!/usr/bin/env python3
"""Regenerates install_metadata.json from the reference impl files (E3-02)."""
import re, glob, json

out = {}
for f in sorted(glob.glob('../../reference/emdash/packages/plugins/src/agents/impl/*/index.ts')):
    src = open(f).read()
    m = re.search(r"id: '([a-zA-Z0-9-]+)'", src)
    pid = m.group(1)
    dep = {}
    npm = re.search(r"npmDependency\(\{\s*id: '[^']+',\s*package: '([^']+)'", src)
    if npm:
        dep['install_type'] = 'npm'
        dep['package'] = npm.group(1)
    else:
        bins = re.search(r"binaryNames: \[([^\]]*)\]", src)
        if bins:
            dep['binaries'] = re.findall(r"'([^']+)'", bins.group(1))
        inst = re.search(r"installCommands: \{\s*macos: \[([^\]]*)\]", src, re.S)
        if inst:
            cmds = inst.group(1)
            m2 = re.search(r"method: '(\w+)',\s*command: '([^']+)',?\s*uninstallCommand: '([^']*)'", cmds)
            m3 = re.search(r"method: '(\w+)',\s*command: '([^']+)'", cmds)
            if m2:
                dep['install_type'] = m2.group(1); dep['command'] = m2.group(2); dep['uninstall_command'] = m2.group(3)
            elif m3:
                dep['install_type'] = m3.group(1); dep['command'] = m3.group(2)
        repo = re.search(r"releaseSource: \{\s*kind: 'github',\s*repo: '([^']+)'", src)
        if repo:
            dep['update_repo'] = repo.group(1)
    out[pid] = dep

json.dump(out, open('install_metadata.json', 'w'), indent=1, sort_keys=True)
print('regenerated', len(out), 'entries')
