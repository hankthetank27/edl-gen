# EDL-Gen #

### Generate an EDL file from live triggers ###  

EDL-Gen is a video broadcast and streaming tool for generating EDL (Edit Decision List) files from custom, mappable, edit "events" synced over a live LTC/SMPTE timecode feed. EzEDL listens for incoming events over a local network using a simple HTTP REST API. When an event request is received, it will parse and log the event metadata (such as AV channels, edit type, tape number, etc.) into an EDL with the corresponding timecode. This allows users to use arbitrary switching software, so long as it can send HTTP requests, to log their live camera switches and automatically import them as edits into their editing software of choice.

## Getting Started ##

EDL-Gen is very much a work in progress at the moment. To get up and running you can build from source. Make sure you have the [Rust Toolchain](https://www.rust-lang.org/tools/install) installed, and run `cargo run` to try it out.

## Documentation ##

Coming soon...

### TODO: ###
- Improve logging (colors formating etc)
- Save settings
- Limit log size
- Handle speed changes
- BitFocus Companion module
