#!/bin/bash
# Analyze duplicate patterns in Brave Nightly

echo "🔍 Analyzing Brave Nightly Bookmarks"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

BOOKMARKS_FILE="$HOME/Library/Application Support/BraveSoftware/Brave-Browser-Nightly/Default/Bookmarks"

if [ ! -f "$BOOKMARKS_FILE" ]; then
    echo "❌ Bookmarks file not found"
    exit 1
fi

echo ""
echo "📊 Statistics:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Count total URLs
TOTAL_URLS=$(grep -o '"url":' "$BOOKMARKS_FILE" | wc -l | tr -d ' ')
echo "Total bookmark entries: $TOTAL_URLS"

# Count unique URLs
UNIQUE_URLS=$(grep -o '"url":"[^"]*"' "$BOOKMARKS_FILE" | sort -u | wc -l | tr -d ' ')
echo "Unique URLs: $UNIQUE_URLS"

# Calculate duplicates
DUPLICATES=$((TOTAL_URLS - UNIQUE_URLS))
echo "Duplicate URLs: $DUPLICATES"

# Calculate percentage
if [ $TOTAL_URLS -gt 0 ]; then
    DUPLICATE_PCT=$(echo "scale=1; ($DUPLICATES * 100) / $TOTAL_URLS" | bc)
    echo "Duplicate rate: ${DUPLICATE_PCT}%"
fi

echo ""
echo "📈 Top 10 Most Duplicated URLs:"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
grep -o '"url":"[^"]*"' "$BOOKMARKS_FILE" | \
    sed 's/"url":"//;s/"$//' | \
    sort | uniq -c | sort -rn | head -10 | \
    while read count url; do
        echo "  $count times: ${url:0:60}..."
    done

echo ""
echo "✅ Analysis complete"
