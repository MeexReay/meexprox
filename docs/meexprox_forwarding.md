# Meexprox Player Forwarding

Meexprox modifies [Handshake](https://wiki.vg/Protocol#Handshake) packet to transfer IP address


| **Packet ID**                            | **State**     | **Bound To** | **Field Name**         | **Field Type**         | **Notes**                                                                                                                                                                                                                                 |
|------------------------------------------|---------------|--------------|-------------------------|-------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| *protocol:* `0x00`<br>*resource:* `intention` | Handshaking  | Server       | **Is IPV6**            | Boolean                 | Indicates if the IP address is using IPV6.                                                                                                                                                                                                |
|                                          |               |              | **IP Octets**          | Byte Array (16)            | IP address represented as octets in bytes. 16 bytes if IPV6 and 4 bytes if IPV4.                                                                                                                                                                                              |
|                                          |               |              | **IP Port**            | Unsigned Short          | IP port number.                                                                                                                                                                                                                          |
|                                          |               |              | **Protocol Version**   | VarInt                  | Protocol version number. Currently 767 in Minecraft 1.21.                                                                                                                                                                               |
|                                          |               |              | **Server Address**     | String (255)            | Hostname or IP, e.g., localhost or 127.0.0.1, used to connect. The server does not use this information. Note: SRV records can redirect; for example, if `_minecraft._tcp.example.com` points to `mc.example.org`.                     |
|                                          |               |              | **Server Port**        | Unsigned Short          | Default is 25565. The server does not use this information.                                                                                                                                                                              |
|                                          |               |              | **Next State**         | VarInt Enum            | Defines the next state: 1 for Status, 2 for Login, 3 for Transfer.                                                                                                                                                                       |

To make it work, download [this plugin](https://github.com/MeexReay/meexprox_plugin) on your backend server

#### Overview
- [Main page](index.md)
- [Player Forwarding](player_forwarding.md)