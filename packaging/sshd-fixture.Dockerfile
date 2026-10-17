# An sshd for the remote-completion test (crates/rift-core/tests/remote_complete.rs):
# key auth only, user `rift` with bash as login shell, a `mycmd` completion
# for bash, zsh and fish, and a git repository in ~/proj. The test builds it, runs it
# with port 22 bound to 127.0.0.1 and its throwaway public key mounted:
#
#   docker build -t rift-sshd-fixture -f packaging/sshd-fixture.Dockerfile packaging
#   docker run -d --rm -p 127.0.0.1::22 -v <dir>/id.pub:/keys/rift.pub:ro rift-sshd-fixture
FROM debian:trixie-slim
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      openssh-server bash zsh fish git ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /run/sshd \
    && useradd -m -s /bin/bash rift \
    && passwd -d rift \
    && printf '%s\n' 'PasswordAuthentication no' 'KbdInteractiveAuthentication no' \
         'PubkeyAuthentication yes' 'AuthorizedKeysFile /keys/%u.pub' 'AcceptEnv LC_*' \
         'MaxSessions 64' 'StrictModes no' > /etc/ssh/sshd_config.d/rift.conf
USER rift
RUN mkdir -p ~/proj/src ~/proj/docs ~/bin \
    && touch ~/proj/remote-only.txt ~/proj/src/main.rs \
    && printf '#!/bin/sh\n' > ~/bin/mycmd && chmod +x ~/bin/mycmd \
    && git -C ~/proj init -q -b main \
    && git -C ~/proj -c user.name=r -c user.email=r@r commit -q --allow-empty -m init \
    && git -C ~/proj branch feature-remote \
    && printf '%s\n' 'export PATH="$HOME/bin:$PATH"' \
         '_mycmd() { COMPREPLY=($(compgen -W "deploy destroy status" -- "$2")); }' \
         'complete -F _mycmd mycmd' >> ~/.bashrc \
    && mkdir -p ~/.zfunc ~/.config/fish/completions \
    && printf '%s\n' '#compdef mycmd' "compadd deploy-zsh destroy-zsh status" > ~/.zfunc/_mycmd \
    && printf '%s\n' 'path=(~/bin $path)' 'fpath=(~/.zfunc $fpath)' \
         'autoload -Uz compinit && compinit -u' > ~/.zshrc \
    && printf '%s\n' 'complete -c mycmd -f -a "deploy-fish destroy-fish status"' \
         > ~/.config/fish/completions/mycmd.fish \
    && printf '%s\n' 'fish_add_path -g ~/bin' > ~/.config/fish/config.fish
USER root
# the key the test mounts (/keys/rift.pub) is the only way in
CMD ["/usr/sbin/sshd", "-D", "-e"]
