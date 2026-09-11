#!/usr/bin/env python3
"""Validate local skill metadata and resource links, not engineering expertise."""
import re
import sys
from pathlib import Path

root = Path(sys.argv[1]) if len(sys.argv) == 2 else Path(__file__).resolve().parents[1] / '.agents/skills'
errors = []
names = set()
documents = sorted(root.glob('*/SKILL.md'))
if not documents:
    errors.append(f'no skills found in {root}')
for document in documents:
    text = document.read_text()
    match = re.match(r'^---\n(.*?)\n---(?:\n|$)', text, re.S)
    fields = dict(re.findall(r'^(name|description):\s*(.+)$', match[1], re.M)) if match else {}
    name = fields.get('name', '')
    if not re.fullmatch(r'[a-z0-9]+(?:-[a-z0-9]+)*', name) or name != document.parent.name:
        errors.append(f'{document}: invalid name or directory mismatch')
    if name in names:
        errors.append(f'{document}: duplicate name {name}')
    names.add(name)
    if not fields.get('description', '').strip(' \"\''):
        errors.append(f'{document}: missing description')
    for resource in document.parent.rglob('*.md'):
        for link in re.findall(r'\]\(([^)]+)\)', resource.read_text()):
            target = link.split('#', 1)[0]
            if not target or re.match(r'[a-zA-Z][a-zA-Z0-9+.-]*:', target):
                continue
            if not (resource.parent / target).exists():
                errors.append(f'{resource}: missing reference {target}')
if errors:
    sys.exit('\n'.join(errors))
print(f'{len(documents)} repository skills: metadata, unique names and references valid')
