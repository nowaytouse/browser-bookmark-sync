#!/usr/bin/env python3
"""修复Safari书签中的嵌套文件夹问题"""
import plistlib
import os
import shutil

SAFARI_PLIST = os.path.expanduser("~/Library/Safari/Bookmarks.plist")
BACKUP_PATH = SAFARI_PLIST + ".nested_fix_backup"

def flatten_nested_folders(node):
    """递归展平嵌套的同名文件夹"""
    if not isinstance(node, dict):
        return node
    
    if node.get('WebBookmarkType') == 'WebBookmarkTypeList':
        name = node.get('Title', '')
        children = node.get('Children', [])
        
        flattened_children = []
        for child in children:
            if isinstance(child, dict):
                child_type = child.get('WebBookmarkType')
                child_name = child.get('Title', '')
                
                if child_type == 'WebBookmarkTypeList' and child_name == name:
                    nested_children = child.get('Children', [])
                    for nested in nested_children:
                        flattened_children.append(flatten_nested_folders(nested))
                else:
                    flattened_children.append(flatten_nested_folders(child))
            else:
                flattened_children.append(child)
        
        node['Children'] = flattened_children
    
    elif 'Children' in node:
        node['Children'] = [flatten_nested_folders(c) for c in node.get('Children', [])]
    
    return node

def main():
    print("🔧 修复Safari嵌套文件夹...")
    shutil.copy2(SAFARI_PLIST, BACKUP_PATH)
    print(f"💾 备份: {BACKUP_PATH}")
    
    with open(SAFARI_PLIST, 'rb') as f:
        data = plistlib.load(f)
    
    fixed_data = flatten_nested_folders(data)
    
    with open(SAFARI_PLIST, 'wb') as f:
        plistlib.dump(fixed_data, f)
    
    print("✅ 修复完成!")
    os.remove(BACKUP_PATH)
    print("🗑️  备份已删除")

if __name__ == "__main__":
    main()
