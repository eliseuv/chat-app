build:
    cargo build --release

deploy APP HOSTNAME: build
    rsync -Pazhvm ./target/release/{{APP}} {{HOSTNAME}}:~/projects/chat-{{APP}}
