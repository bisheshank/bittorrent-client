# Introduction

Our project involved writing a barebones bittorrent client. We knew going into the project that it was an ambitious task to take on, so we tried to simplify it as much as possible without making the project trivial. Our goal was for the client to have four basic functionalities:

- Parse a local .torrent file and obtain necessary metadata required to download the content
- Connect to tracker and peers via TCP
- Download content from peers in pieces a failsafe manner
- Act as a seeder after completing the download

In the end, we were able to achieve the first three goals. Our client is a command line call with the .torrent file as its argument, and it displays download progress and important information such as peer count and their addresses. We ended up running out of time to do seeding, but we learned a ton just from completing a leeching client.

# Design/Implementation

### Parsing .torrent files

Our client uses the Serde library to decode .torrent bencode files that contain

- Tracker (announce) URL
- File information like the name, total length, and piece length
- Info hashes for each piece to verify validity of data received from peers
- File mode: single or multi

An example of the contents of a torrent file:

```
Torrent {
    announce: "my-announce-url.com/announce",
    info: Info {
        name: "file.txt",
        piece_length: 32768,
        pieces: PieceHashes([[Hash of piece 1], [Hash of piece 2], [Hash of piece 3],...]),
        files: SingleFile { length: 92063 }
    },
    info_hash: Some([214, 159, 145, 230, 178, 174, 76, 84, 36, 104, 209, 7, 58, 113, 212, 234, 19, 135, 154, 127])
}
```

Source code: from_file in [torrent.rs](./src/torrent.rs)

### Communicating With Trackers

To begin the download process, our client sends a request to the tracker url to retrieve the list of peers.
