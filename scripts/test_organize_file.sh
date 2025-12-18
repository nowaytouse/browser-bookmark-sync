#!/bin/bash
# Test organize from file functionality

set -e

echo "🧪 Testing organize from file functionality..."

# Create test bookmark file
cat > /tmp/test_bookmarks.html << 'EOF'
<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><A HREF="https://github.com/user/repo">GitHub Project</A>
    <DT><A HREF="https://twitter.com/user">Twitter Profile</A>
    <DT><A HREF="https://bilibili.com/video/123">Bilibili Video</A>
    <DT><A HREF="https://v2ray.com/docs">V2Ray Docs</A>
    <DT><A HREF="https://binance.com/trade">Binance Trade</A>
    <DT><A HREF="https://naver.com/news">Naver News</A>
    <DT><A HREF="https://pixiv.net/artworks/123">Pixiv Artwork</A>
    <DT><A HREF="https://example.com/random">Random Page</A>
</DL><p>
EOF

echo "📄 Created test bookmark file: /tmp/test_bookmarks.html"

# Test dry-run with stats
echo ""
echo "📊 Testing dry-run with stats..."
./target/release/browser-bookmark-sync organize \
    --file /tmp/test_bookmarks.html \
    --output /tmp/test_organized.html \
    --stats \
    --dry-run

# Test actual organize
echo ""
echo "🔧 Testing actual organize..."
./target/release/browser-bookmark-sync organize \
    --file /tmp/test_bookmarks.html \
    --output /tmp/test_organized.html \
    --stats

# Verify output file exists
if [ -f /tmp/test_organized.html ]; then
    echo ""
    echo "✅ Output file created successfully!"
    echo "📄 Output preview:"
    head -30 /tmp/test_organized.html
else
    echo "❌ Output file not created!"
    exit 1
fi

# Cleanup
rm -f /tmp/test_bookmarks.html /tmp/test_organized.html

echo ""
echo "✅ All organize file tests passed!"
