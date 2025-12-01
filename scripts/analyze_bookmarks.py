#!/usr/bin/env python3
"""分析书签数据"""
import json
import os
from collections import Counter
from urllib.parse import urlparse

BACKUP_FILE = os.path.expanduser("~/Library/Safari/MasterBackup/unique_bookmarks.json")

def extract_domain(url):
    try:
        parsed = urlparse(url)
        domain = parsed.netloc.lower()
        if domain.startswith('www.'):
            domain = domain[4:]
        return domain
    except:
        return 'unknown'

with open(BACKUP_FILE, 'r', encoding='utf-8') as f:
    bookmarks = json.load(f)

print(f"总书签数: {len(bookmarks)}")

# 统计域名
domains = Counter()
for bm in bookmarks:
    url = bm.get('url', '')
    domain = extract_domain(url)
    domains[domain] += 1

print("\n=== 域名 Top 50 ===\n")
for i, (domain, count) in enumerate(domains.most_common(50), 1):
    print(f"{i:3d}. {domain}: {count}")

# 托管平台
print("\n=== 托管平台 ===")
hosting = {'github.io': 0, 'vercel.app': 0, 'netlify.app': 0, 'pages.dev': 0, 'neocities.org': 0}
for domain in domains:
    for h in hosting:
        if domain.endswith(h):
            hosting[h] += domains[domain]
for h, c in sorted(hosting.items(), key=lambda x: -x[1]):
    if c > 0:
        print(f"  *.{h}: {c}")
