cargo build
cd worker
cargo build
cd ..
cd target/debug
rm -r fonts
ln -s ../../fonts fonts
export DISCORD_TOKEN=$(cat ../../token.secret)
./typst-bot