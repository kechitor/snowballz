#!/bin/sh
# Adds Snowballz to your applications menu (run it again if you move this
# folder; delete ~/.local/share/applications/snowballz.desktop to remove it).
DIR=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$HOME/.local/share/applications"
cat > "$HOME/.local/share/applications/snowballz.desktop" <<EOD
[Desktop Entry]
Type=Application
Name=Snowballz
Comment=Snowball fight strategy game with penguins
Exec="$DIR/snowballz"
Path=$DIR
Icon=$DIR/snowballz.png
Terminal=false
Categories=Game;StrategyGame;
EOD
echo "Added Snowballz to the applications menu."
