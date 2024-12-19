build:
    cargo build --release

deploy HOSTNAME: build
    rsync -Pazhvm ./target/release/chat-app {{HOSTNAME}}:~/projects/
