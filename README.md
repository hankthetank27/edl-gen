# EDLgen #

### Generate EDL files from live triggers

EDLgen is a video broadcast, streaming and editing tool for generating EDL (Edit Decision List) files from custom, mappable, "edit events" synced over a live LTC/SMPTE timecode feed. EDLgen listens for incoming events over a network using a simple HTTP REST API. When an event request is received, it will log the event metadata (such as AV channels, edit type, tape number, etc.) into an EDL file with the corresponding timecode for the edit. This allows users to use arbitrary switching software or hardware, so long as it's capable of sending HTTP requests to log their live camera switches and automatically import them as edits into their editing software of choice.

## Installation ##

### From Package ###

The most recent pre-built binaries can be found [here](https://github.com/hankthetank27/edl-gen/releases/latest).

### From Source ###

Make sure you have the [Rust Toolchain](https://www.rust-lang.org/tools/install) installed, and run `cargo run` to try it out quickly.

Windows builds use ASIO drivers with [cpal](https://github.com/RustAudio/cpal). Setup instructions can be found [here](https://github.com/RustAudio/cpal?tab=readme-ov-file#asio-on-windows).

## Usage

### Overview

EGLgen works by listening to an LTC/SMPTE timecode source signal via an audio device while also listening for edit events via an HTTP server over a local network. 

When you launch EDLgen, you'll be displayed a window which contains some configuration options for the project, and some controls to start and stop the server along with an area below which displays information about edit events captured and other server logs. One you configure you project to the desired settings, you can launch the server to start listening for edit events to write to an EDL.

EDLgen's output EDL conforms to the [CMX3600 specification](https://www.edlmax.com/EdlMaxHelp/Edl/maxguide.html). 

Edit events in are received the form of HTTP requests made to the configured port, and should contain a payload specifying event data such as edit type and tape number (more detailed event API docs can be found below). When the EDLgen server receives an event request, it will write the edit data as described in the event request payload to an EDL file with the given project name.


### Configuration and Controls

- **Project Name**: Sets the name of the EDL file that will be written to after the first `START` event is received. EDLgen will never overwrite an existing EDL with the same name as the given project name in the same storage directory. Rather, it will append a number to the end of the file name. Ex. `my-video.edl` would be written as `my-video(1).edl` if a file of that name already existed.

- **Storage Directory**: Sets the directory/folder where the EDL output file will be stored.

- **Audio Device**: Sets the audio input device where the timecode input is expected. 

- **Refresh Devices**: Refreshes the list of available audio devices.

- **Input Channel**: Sets the input channel on the selected audio device where the timecode signal is expected.

- **Buffer Size**: Sets the buffer size for the selected audio device when listening to the timecode. The higher the buffer size, the more latency between the edit even and the logging. This is set to 1024 (or the next highest available) by default as this is what has worked best on my machine when testing.

- **LTC Input Sample Rate**: Sets what the sample rate of the incoming timecode/LTC signal should be expected to be for decoding purposes. This setting does not change the sample rate of the selected audio device.

- **Frame Rate**: Sets the expected frame rate of the input timecode for decoding purposes.

- **NTSC/FCM**: Sets whether the input timecode is expected to be drop frame or non-drop frame.

- **TCP Port**: Sets the port number the even server will be listening on. See API documentation below for more details.

- **Launch Server**: Launches the HTTP server and beginnings listening for edit events using the configured settings. Once the server has been launched you must close it to reconfigure your settings.

- **Stop Server**: Closes the server if already launched, allowing you to reconfigure your settings. 

### Triggering Edit Events / API

The event trigger API describes how the EDLgen server expects to receive events, and what type of metadata the events and ingest and log. 

To trigger an edit event, an HTTP POST request must be sent to the configured TCP port number, with a JSON payload containing the event metadata. For instance if you configured your port to be 9000, over your local network you would ping `127.0.0.1:9000/{even_name_here}`.

#### Edit Events

- **START** - POST to `127.0.0.1:{port_num}/start` - Triggers the creation of a new EDL file, the initialization of the LTC timecode decoding process, and the first edit log in the EDL. If there is no timecode signal present, the event will wait until a signal is detected before proceeding with logging, meaning you can trigger a start event before you actually start playback of your source. No subsequent events can be triggered until a **START** event has been received.

- **LOG** - POST to `127.0.0.1:{port_num}/log` - Triggers the logging of an edit once the EDL has been created and the LTC is decoding after a **START** event as been received. This will likely be the most used event type unless you plan on logging a single edit. 

- **END** - POST to `127.0.0.1:{port_num}/end` - Triggers the logging of the final edit in the EDL. Once this event is received the EDL file will be closed, and you can trigger a **START** event again to create a new EDL if desired.

##### Edit Event JSON Metadata
Each event type expects the same JSON payload structure:

```typescript
{
    "edit_type": "cut" | "wipe" | "dissolve",
    "edit_duration_frames"?: number, 
    "wipe_num"?: number,
    "source_tape"?: string,   
    "av_channels": {     
        "video": boolean,     
        "audio": number   
    } 
}
```
- `edit_type`: Specifies what the edit type should be - either a cut, a wipe or a dissolve. If the edit type is a dissolve or a wipe, a duration in required in the `edit_duration_frames` field. Wipes can also optionally have a wipe number which can tell the editing system which wipe to use. This is specified in the `wipe_num` field.

- `edit_duration_frames`: Specifies the length of the edit in frames. This value is required for dissolves and wipes. For cuts it is ignored.

- `wipe_num`: Optionally specifies which wipe should be used by the editing system (defaults to `1`). This value is ignored for cuts and dissolves.

- `source_tape`: Optionally specifies the name of the of the tape the edit is being made for. This typically would be the name of the file the source of the video will correspond with in your editing software. The file extension might be needed in such a case depending on the editing software you use. If this filed is not included, EDLgen will attempt to use the preselected source tape which is set by the [**SELECT SOURCE** event](#other-events).

- `av_channels`: Specifies the video and audio channels.
    - `video`: Specifies if the channel contains video.
    - `audio`: Specifies the number of audio channels.


Examples...
```json
{   
    "edit_type": "cut",   
    "source_tape": "clip2",   
    "av_channels": {     
        "video": true,     
        "audio": 2   
    } 
}
```
```json
{   
    "edit_type": "wipe",
    "edit_duration_frames": 18,
    "wipe_num": 19,
    "source_tape": "clip1.mp4",
    "av_channels": {
        "video": true,
        "audio": 2   
    } 
}
```
#### Other Events

- **SELECT SOURCE** - POST to `127.0.0.1:{port_num}/select-src` - Triggers the pre-selection of a source tape to be used by the next edit events, and will be utilized when an edit event does not contain a `source_tape` field.

##### Select Source Event JSON Metadata

```typescript
{
    "source_tape": string,   
}
```
- `source_tape`: Specifies the name of the of the tape the edit is being made for.

### Planned Features and TODO
- Handle speed changes
- Improved logging 
    - Colors
    - (Dev) Render scroll area with egui rows
    - (Dev) Limit log size?
