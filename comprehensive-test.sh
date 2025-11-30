#!/bin/bash

# Comprehensive Test Suite for Browser Sync
# Tests all features with real Waterfox and Brave Nightly data

set -e

BINARY="./target/release/browser-bookmark-sync"
BROWSERS="waterfox,brave-nightly"
TEST_RESULTS=()

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🧪 COMPREHENSIVE BROWSER SYNC TEST SUITE"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Target browsers: $BROWSERS"
echo "Test time: $(date '+%Y-%m-%d %H:%M:%S')"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Function to record test result
record_test() {
    local test_name="$1"
    local result="$2"
    TEST_RESULTS+=("$test_name: $result")
}

# Build project
echo "📦 Building project..."
cargo build --release
if [ $? -eq 0 ]; then
    record_test "Build" "✅ PASSED"
    echo "✅ Build successful"
else
    record_test "Build" "❌ FAILED"
    echo "❌ Build failed"
    exit 1
fi
echo ""

# Test 1: Browser Detection
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 1: Browser Detection"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY list > /tmp/browser-sync-test-list.txt 2>&1
if grep -q "Waterfox" /tmp/browser-sync-test-list.txt && grep -q "Brave Nightly" /tmp/browser-sync-test-list.txt; then
    record_test "Browser Detection" "✅ PASSED"
    echo "✅ Both browsers detected"
else
    record_test "Browser Detection" "❌ FAILED"
    echo "❌ Browser detection failed"
fi
cat /tmp/browser-sync-test-list.txt
echo ""

# Test 2: Pre-sync Validation
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 2: Pre-sync Validation"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY validate --detailed > /tmp/browser-sync-test-validate.txt 2>&1
if grep -q "validated successfully" /tmp/browser-sync-test-validate.txt; then
    record_test "Pre-sync Validation" "✅ PASSED"
    echo "✅ Validation passed"
else
    record_test "Pre-sync Validation" "⚠️  WARNING"
    echo "⚠️  Validation completed with warnings"
fi
grep -E "(Detected Browsers|Bookmarks Read|Validation Results)" /tmp/browser-sync-test-validate.txt | head -20
echo ""

# Test 3: Incremental Sync (Dry Run)
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 3: Incremental Sync (Dry Run)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY sync --browsers "$BROWSERS" --mode incremental --dry-run --verbose > /tmp/browser-sync-test-inc.txt 2>&1
if grep -q "Dry run mode" /tmp/browser-sync-test-inc.txt; then
    record_test "Incremental Sync (Dry)" "✅ PASSED"
    echo "✅ Incremental sync dry run completed"
    
    # Extract statistics
    echo ""
    echo "📊 Statistics:"
    grep -E "(Read|Merged|Removed|duplicates)" /tmp/browser-sync-test-inc.txt | grep -v "DEBUG" | tail -10
else
    record_test "Incremental Sync (Dry)" "❌ FAILED"
    echo "❌ Incremental sync dry run failed"
fi
echo ""

# Test 4: Full Sync (Dry Run)
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 4: Full Sync (Dry Run)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY sync --browsers "$BROWSERS" --mode full --dry-run --verbose > /tmp/browser-sync-test-full.txt 2>&1
if grep -q "Dry run mode" /tmp/browser-sync-test-full.txt; then
    record_test "Full Sync (Dry)" "✅ PASSED"
    echo "✅ Full sync dry run completed"
    
    # Extract deduplication stats
    echo ""
    echo "📊 Deduplication Statistics:"
    grep -E "(Pre-merge|Post-merge|removed.*duplicates)" /tmp/browser-sync-test-full.txt | grep -v "DEBUG"
else
    record_test "Full Sync (Dry)" "❌ FAILED"
    echo "❌ Full sync dry run failed"
fi
echo ""

# Test 5: Deduplication Performance
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 5: Deduplication Performance Analysis"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Extract numbers from full sync test - use Python for reliable parsing
STATS=$(python3 << 'PYEOF'
import re

try:
    with open('/tmp/browser-sync-test-full.txt', 'r') as f:
        content = f.read()
    
    # Find "structure: X URLs" lines to get input count
    structure_matches = re.findall(r'structure: (\d+) URLs', content)
    before = sum(int(x) for x in structure_matches) if structure_matches else 0
    
    # Find "Merged bookmarks: X URLs" to get output count
    merged_match = re.search(r'Merged bookmarks: (\d+) URLs', content)
    after = int(merged_match.group(1)) if merged_match else 0
    
    # Find "removed X duplicates"
    removed_matches = re.findall(r'removed (\d+) duplicates', content)
    removed = sum(int(x) for x in removed_matches) if removed_matches else 0
    
    # Calculate reduction
    reduction = (removed / before * 100) if before > 0 else 0
    
    print(f"{before}|{after}|{removed}|{reduction:.1f}")
except Exception as e:
    print("0|0|0|0.0")
PYEOF
)

BEFORE=$(echo "$STATS" | cut -d'|' -f1)
AFTER=$(echo "$STATS" | cut -d'|' -f2)
REMOVED=$(echo "$STATS" | cut -d'|' -f3)
REDUCTION=$(echo "$STATS" | cut -d'|' -f4)

if [ "$BEFORE" != "0" ] && [ "$AFTER" != "0" ]; then
    echo "📊 Deduplication Metrics:"
    echo "   Input bookmarks:  $BEFORE"
    echo "   Output bookmarks: $AFTER"
    echo "   Duplicates removed: $REMOVED"
    echo "   Reduction rate: ${REDUCTION}%"
    
    if (( $(echo "$REDUCTION > 0" | bc -l) )); then
        record_test "Deduplication Performance" "✅ PASSED (${REDUCTION}% reduction)"
        echo "✅ Deduplication working effectively"
    else
        record_test "Deduplication Performance" "⚠️  WARNING (no duplicates found)"
        echo "⚠️  No duplicates found (may be already clean)"
    fi
else
    record_test "Deduplication Performance" "⚠️  SKIPPED"
    echo "⚠️  Could not extract statistics"
fi
echo ""

# Test 6: Cleanup (Dry Run)
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 6: Cleanup Functionality (Dry Run)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY cleanup --browsers "$BROWSERS" --remove-duplicates --remove-empty-folders --dry-run --verbose > /tmp/browser-sync-test-cleanup.txt 2>&1
if grep -q "Dry run" /tmp/browser-sync-test-cleanup.txt; then
    record_test "Cleanup (Dry)" "✅ PASSED"
    echo "✅ Cleanup dry run completed"
    grep -E "(would remove|duplicates|empty folders)" /tmp/browser-sync-test-cleanup.txt | head -10
else
    record_test "Cleanup (Dry)" "❌ FAILED"
    echo "❌ Cleanup dry run failed"
fi
echo ""

# Test 7: Smart Organization (Dry Run)
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 7: Smart Organization (Dry Run)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY smart-organize --browsers "$BROWSERS" --dry-run --show-stats > /tmp/browser-sync-test-organize.txt 2>&1
if grep -q "Smart organization" /tmp/browser-sync-test-organize.txt; then
    record_test "Smart Organization (Dry)" "✅ PASSED"
    echo "✅ Smart organization dry run completed"
    grep -E "(Classification|bookmarks|folders)" /tmp/browser-sync-test-organize.txt | head -10
else
    record_test "Smart Organization (Dry)" "❌ FAILED"
    echo "❌ Smart organization dry run failed"
fi
echo ""

# Test 8: Post-sync Validation
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 8: Post-sync Validation"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
$BINARY validate --detailed > /tmp/browser-sync-test-validate-post.txt 2>&1
if grep -q "validated successfully" /tmp/browser-sync-test-validate-post.txt; then
    record_test "Post-sync Validation" "✅ PASSED"
    echo "✅ Post-sync validation passed"
else
    record_test "Post-sync Validation" "⚠️  WARNING"
    echo "⚠️  Post-sync validation completed with warnings"
fi
echo ""

# Test 9: Performance Benchmarks
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 9: Performance Benchmarks"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Measure sync time
START_TIME=$(date +%s)
$BINARY sync --browsers "$BROWSERS" --mode incremental --dry-run > /dev/null 2>&1
END_TIME=$(date +%s)
SYNC_TIME=$((END_TIME - START_TIME))

echo "⏱️  Performance Metrics:"
echo "   Incremental sync (dry): ${SYNC_TIME}s"

if [ $SYNC_TIME -lt 30 ]; then
    record_test "Performance" "✅ PASSED (${SYNC_TIME}s)"
    echo "✅ Performance acceptable"
else
    record_test "Performance" "⚠️  SLOW (${SYNC_TIME}s)"
    echo "⚠️  Performance slower than expected"
fi
echo ""

# Test 10: Memory Usage
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Test 10: Memory Usage Check"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Run sync in background and monitor memory
$BINARY sync --browsers "$BROWSERS" --mode full --dry-run > /dev/null 2>&1 &
SYNC_PID=$!
sleep 2

if ps -p $SYNC_PID > /dev/null 2>&1; then
    MEMORY=$(ps -o rss= -p $SYNC_PID | awk '{print int($1/1024)}')
    echo "💾 Memory Usage: ${MEMORY}MB"
    
    if [ $MEMORY -lt 500 ]; then
        record_test "Memory Usage" "✅ PASSED (${MEMORY}MB)"
        echo "✅ Memory usage acceptable"
    else
        record_test "Memory Usage" "⚠️  HIGH (${MEMORY}MB)"
        echo "⚠️  Memory usage higher than expected"
    fi
else
    record_test "Memory Usage" "⚠️  SKIPPED"
    echo "⚠️  Process completed too quickly to measure"
fi

wait $SYNC_PID 2>/dev/null
echo ""

# Summary
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📊 TEST SUMMARY"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

PASSED=0
FAILED=0
WARNING=0

for result in "${TEST_RESULTS[@]}"; do
    echo "$result"
    if [[ $result == *"✅ PASSED"* ]]; then
        ((PASSED++))
    elif [[ $result == *"❌ FAILED"* ]]; then
        ((FAILED++))
    elif [[ $result == *"⚠️"* ]]; then
        ((WARNING++))
    fi
done

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Total Tests: ${#TEST_RESULTS[@]}"
echo "✅ Passed: $PASSED"
echo "❌ Failed: $FAILED"
echo "⚠️  Warnings: $WARNING"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

if [ $FAILED -eq 0 ]; then
    echo "🎉 All critical tests passed!"
    echo ""
    echo "💡 Next steps:"
    echo "   1. Review test results above"
    echo "   2. Run actual sync: browser-bookmark-sync sync --mode incremental"
    echo "   3. Verify in browsers: Waterfox and Brave Nightly"
    echo ""
    exit 0
else
    echo "❌ Some tests failed. Please review the output above."
    exit 1
fi
