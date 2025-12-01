#!/usr/bin/env python3
import sqlite3
import plistlib
import uuid
from pathlib import Path

wf_db = Path.home() / "Library/Application Support/Waterfox/Profiles/94cley6c.default-release-1757404159134/places.sqlite"
conn = sqlite3.connect(str(wf_db))

def get_children(parent_id):
    cur = conn.execute("""
        SELECT b.id, b.type, b.title, p.url
        FROM moz_bookmarks b LEFT JOIN moz_places p ON b.fk = p.id
        WHERE b.parent = ? ORDER BY b.position
    """, (parent_id,))
    items = []
    for row in cur:
        bid, btype, title, url = row
        if btype == 2:
            items.append({
                'WebBookmarkType': 'WebBookmarkTypeList',
                'Title': title or 'Folder',
                'WebBookmarkUUID': str(uuid.uuid4()).upper(),
                'Children': get_children(bid)
            })
        elif btype == 1 and url:
            items.append({
                'WebBookmarkType': 'WebBookmarkTypeLeaf',
                'URLString': url,
                'URIDictionary': {'title': title or url[:50]},
                'WebBookmarkUUID': str(uuid.uuid4()).upper(),
            })
    return items

def cnt(items):
    b, f = 0, 0
    for i in items:
        if i['WebBookmarkType'] == 'WebBookmarkTypeLeaf': b += 1
        else: f += 1; cb, cf = cnt(i.get('Children', [])); b += cb; f += cf
    return b, f

root = conn.execute("SELECT id FROM moz_bookmarks WHERE parent=0 OR parent IS NULL").fetchone()[0]
bookmarks = get_children(root)
b, f = cnt(bookmarks)
print(f"Waterfox: {b} 书签, {f} 文件夹")

safari_path = Path.home() / "Library/Safari/Bookmarks.plist"
import shutil
shutil.copy(safari_path, safari_path.with_suffix('.plist.bak3'))

data = {
    'WebBookmarkFileVersion': 1,
    'WebBookmarkType': 'WebBookmarkTypeList',
    'WebBookmarkUUID': str(uuid.uuid4()).upper(),
    'Children': [
        {'WebBookmarkType': 'WebBookmarkTypeList', 'Title': 'BookmarksBar',
         'WebBookmarkUUID': str(uuid.uuid4()).upper(), 'Children': bookmarks},
        {'WebBookmarkType': 'WebBookmarkTypeList', 'Title': 'BookmarksMenu',
         'WebBookmarkUUID': str(uuid.uuid4()).upper(), 'Children': []}
    ]
}

with open(safari_path, 'wb') as f:
    plistlib.dump(data, f)
print(f"✅ Safari已更新: {b} 书签, {f} 文件夹")
conn.close()
