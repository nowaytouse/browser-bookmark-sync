#!/opt/homebrew/bin/bash

# Browser Bookmark Sync - Quick Start Guide

echo "🚀 Browser Bookmark Sync - Quick Start"
echo "======================================"
echo ""

# Build the project
echo "📦 Building project..."
cargo build --release

if [ $? -ne 0 ]; then
    echo "❌ Build failed!"
    exit 1
fi

echo "✅ Build successful!"
echo ""

# Set up alias
BINARY_PATH="$(pwd)/target/release/browser-bookmark-sync"

echo "📋 Available commands:"
echo ""
echo "1. List detected browsers:"
echo "   $BINARY_PATH list"
echo ""
echo "2. Validate bookmarks:"
echo "   $BINARY_PATH validate"
echo ""
echo "3. Dry run sync (preview only):"
echo "   $BINARY_PATH sync --dry-run"
echo ""
echo "4. Perform actual sync:"
echo "   $BINARY_PATH sync"
echo ""
echo "5. Schedule automatic sync (every 30 minutes):"
echo "   $BINARY_PATH schedule"
echo ""
echo "6. Schedule with custom cron (every hour):"
echo "   $BINARY_PATH schedule --cron '0 0 * * * *'"
echo ""

# Offer to run list command
echo "Would you like to list detected browsers now? (y/n)"
read -r response

if [[ "$response" =~ ^[Yy]$ ]]; then
    echo ""
    echo "🔍 Detecting browsers..."
    $BINARY_PATH list
fi

echo ""
echo "✨ Setup complete! You can now use the commands above."
