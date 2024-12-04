# EDLgen #

### Generate EDL files from live triggers over HTTP ###  

EDLgen is a video broadcast, streaming and editing tool for generating EDL (Edit Decision List) files from custom, mappable, edit "events" synced over a live LTC/SMPTE timecode feed. EDLgen listens for incoming events over a network using a simple HTTP REST API. When an event request is received, it will log the event metadata (such as AV channels, edit type, tape number, etc.) into an EDL file with the corresponding timecode for the edit. This allows users to use arbitrary switching software or hardware, so long as it's capable of sending HTTP requests to log their live camera switches and automatically import them as edits into their editing software of choice.

## Getting Started ##

EDLgen is very much a work in progress at the moment. To get up and running you can build from source. Make sure you have the [Rust Toolchain](https://www.rust-lang.org/tools/install) installed, and run `cargo run` to try it out.

## Documentation ##

Coming soon...

### TODO: ###
- Save settings
- BitFocus Companion module
- Improve logging 
    - Colors, formating etc
    - Render scroll area with egui rows
    - Limit log size
- Handle speed changes
