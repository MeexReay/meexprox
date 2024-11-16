# Player Forwarding

Player Forwarding is a protocol that describes how to pass player information (name, uuid, ip, properties, etc.) to backend server from proxy server

Config of player forwarding:

```yml
forwarding:
  enabled: false   # is player forwarding enabled
  type: velocity   # player forwarding type
  secret: "123456" # player forwarding secret key
```

### Player forwarding types

- `meexprox` - meexprox player forwarding ([about it](meexprox_forwarding.md)) ([plugin](https://github.com/MeexReay/meexprox_plugin))
- `velocity` - velocity 'modern' player forwarding, secret key is required
- `bungeecord` (with secret) - bungeecord player forwarding
- `bungeecord` (without secret) - bungeeguard player forwarding

#### Overview
- [Main page](index.md)
- [Player Forwarding](player_forwarding.md)