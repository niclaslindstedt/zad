# Telegram setup

Runnable lifecycle walkthrough for the Telegram service. The runtime
verbs (`zad telegram send`, `read`, …) are still being implemented —
this example exercises only the lifecycle surface, which is complete.

## Register credentials

Grab a bot token from [@BotFather](https://t.me/BotFather) on Telegram
and export it:

```sh
export TELEGRAM_BOT_TOKEN='123456789:ABC-DEF1234ghIkl-zyx57W2v1u123ew11'
```

Register it globally so every project on this machine can reuse it:

```sh
zad service create telegram \
    --bot-token-env TELEGRAM_BOT_TOKEN \
    --default-chat '@my_channel' \
    --scopes messages.read,messages.send \
    --non-interactive
```

Or, if this project should own its own bot:

```sh
zad service create telegram --local \
    --bot-token-env TELEGRAM_BOT_TOKEN \
    --scopes messages.read,messages.send \
    --non-interactive
```

## Enable in the project

```sh
cd ~/code/my-project
zad service enable telegram
zad service list            # should show telegram = enabled
zad service show telegram   # should show which scope won + token presence
```

## Sample global config file

The file `config.toml` in this directory is a representative sample of
what `zad service create telegram` writes to
`~/.zad/services/telegram/config.toml`. The bot token itself is **not**
in this file — it lives in the OS keychain under
`service="zad", account="telegram-bot:global"`.

## Tear down

```sh
zad service disable telegram          # remove the [service.telegram] entry
zad service delete telegram           # clear the global config + keychain entry
zad service delete telegram --local   # or the project-local counterparts
```
