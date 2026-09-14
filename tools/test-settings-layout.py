#!/usr/bin/env python3
"""Validate rendered Settings HTML (BOREAL_UI_FIXTURE_DIR from the Rust fixture test)."""
from html.parser import HTMLParser
import sys


class SettingsLayout(HTMLParser):
    def __init__(self):
        super().__init__()
        self.divs = []
        self.modals = set()
        self.labels = set()

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == 'label':
            self.labels.add(attrs.get('for'))
        if tag != 'div':
            return
        modal = 'modal' in attrs.get('class', '').split()
        if modal:
            ancestor = next((item for item in self.divs if item[1]), None)
            assert ancestor is None, f"Dialog {attrs.get('id')} is inside hidden dialog {ancestor}"
            assert attrs.get('id') not in self.modals, 'Duplicate dialog ID'
            self.modals.add(attrs.get('id'))
        self.divs.append((attrs.get('id'), modal))

    def handle_endtag(self, tag):
        if tag == 'div':
            assert self.divs, 'Extra closing div'
            self.divs.pop()


parser = SettingsLayout()
with open(sys.argv[1]) as source:
    parser.feed(source.read())
assert not parser.divs, 'Unclosed layout divs'
assert {'googleSetupModal', 'googleClientSetupModal', 'githubTokenSetupModal', 'keeperSetupModal',
        'service-s3', 'service-google-drive', 'service-persons'} <= parser.modals
assert 'service-google-groups' not in parser.modals
assert {'s3Enabled', 's3RemoteName'} <= parser.labels
print('PASS: every Settings dialog is independent; layout tags are balanced and S3 controls have labels.')
