# EDLgen #

### Generate EDL files from live triggers

EDLgen is a video broadcast, streaming and editing tool for generating EDL (Edit Decision List) files from custom, mappable, "edit events" synced over a live LTC/SMPTE timecode feed. EDLgen listens for incoming events over a network using a simple HTTP REST API. When an event request is received, it will log the event data (such as AV channels, edit type, tape number, etc.) into an EDL file with the corresponding timecode for the edit. This allows users to use arbitrary switching software or hardware, so long as it's capable of sending HTTP requests to log their live camera switches and automatically import them as edits into their editing software of choice.

## Installation ##

### From Package ###

The most recent pre-built binaries can be found [here](https://github.com/hankthetank27/edl-gen/releases/latest).

### From Source ###

Make sure you have the [Rust Toolchain](https://www.rust-lang.org/tools/install) installed, and run `cargo run` to try it out quickly.

Windows builds use ASIO drivers with [cpal](https://github.com/RustAudio/cpal). Setup instructions can be found [here](https://github.com/RustAudio/cpal?tab=readme-ov-file#asio-on-windows).

## Usage

### Overview

EGLgen works by listening to an LTC/SMPTE timecode source signal via an audio device while also listening for edit events via an HTTP server over a local network. 

When you launch EDLgen, you'll be displayed a window which contains some configuration options for the project and some controls to start and stop the server along with an area below which logs information about edit events captured. Once you configure you project to the desired settings, you can launch the server to start listening for edit events to write to an EDL.

EDLgen's output EDL conforms to the [CMX3600 specification](https://www.edlmax.com/EdlMaxHelp/Edl/maxguide.html). 

Edit events in are received the form of HTTP requests made to the configured port and should contain a payload specifying event data such as edit type and tape number (more detailed event API docs can be found below). When the EDLgen server receives an event request, it will write the edit data as described in the event request payload to an EDL file.


### Configuration and Controls

- **Project Name**: Sets the name of the EDL file that will be written to after the first `START` event is received. EDLgen will never overwrite an existing EDL with the same name as the given project name in the same storage directory. Rather, it will append a number to the end of the file name. Ex. `my-video.edl` would be written as `my-video(1).edl` if a file of that name already existed.

- **Storage Directory**: Sets the directory/folder where the EDL output file will be stored.

- **Audio Device**: Sets the audio input device where the timecode input is expected. 

- **Refresh Devices**: Refreshes the list of available audio devices.

- **Input Channel**: Sets the input channel on the selected audio device where the timecode signal is expected.

- **Buffer Size**: Sets the buffer size for the selected audio device when listening to the timecode. The higher the buffer size, the more latency between the edit even and the logging. This is set to 1024 (or the next highest available) by default as this is what has worked best in testing.

- **LTC Input Sample Rate**: Sets what the sample rate of the incoming timecode/LTC signal should be expected to be for decoding purposes. This setting does not change the sample rate settings of the selected audio device.

- **Frame Rate**: Sets the expected frame rate of the input timecode for decoding purposes.

- **NTSC/FCM**: Sets whether the input timecode is expected to be drop frame or non-drop frame.

- **TCP Port**: Sets the port number the even server will be listening on. See API documentation below for more details.

- **Launch Server**: Launches the HTTP server and beginnings listening for edit events using the configured settings. Once the server has been launched you must close it to reconfigure your settings.

- **Stop Server**: Closes the server if already launched, allowing you to reconfigure your settings. 

### Triggering Edit Events / API

The event trigger API describes how the EDLgen server expects to receive events and what type of data the events ingest and log.

To trigger an edit event, an HTTP request must be sent to the configured TCP port number, with a JSON payload containing the event data. For instance if you configured your port to be 9000, over your local network you would ping `127.0.0.1:9000/{even_name_here}`.

#### Edit Events

Edit events are triggered from HTTP POST requests to the below endpoints. Each edit event request responds with the resulting [recording state](#recording-state) and information about the logged edit, if applicable. The response body for each of these events looks something like this:

```typescript
{
    "recording_state": "started" | "stopped" | "waiting",
    "edit": null | {...},
    "final_edits": null | [{...}, ...] 
}
```

- **START** - POST to `127.0.0.1:{port_num}/start` - Triggers the creation of a new EDL file, the initialization of the LTC timecode decoding process, and the first edit log in the EDL. If there is no timecode signal present, the event will wait until a signal is detected before proceeding with logging, meaning you can trigger a start event before you actually start playback of your source. No subsequent events can be triggered until a **START** event has been received and LTC decoding has started (ie, the "started" recording state). This event responds with a `null` value in the `edit` and `final_edits` fields, as an edit is constructed from two events; an in and out point.

- **LOG** - POST to `127.0.0.1:{port_num}/log` - Triggers the logging of an edit once the EDL has been created and the LTC is decoding after a **START** event as been received and the LTC decoding has begun. This event responds with information about the edit built in the `edit` field. This edit data is built from the prior edits out point timecode and source tape, and ends with the current timecode. 

- **END** - POST to `127.0.0.1:{port_num}/end` - Triggers the logging of the final edit in the EDL. Once this event is received the EDL file will be closed, and you can trigger a **START** event again to create a new EDL if desired. This event does not require (and in fact ignores) any values in the `source_tape` and `edit_type` fields, as the last edit cuts to black. This event responds with information about either one or two edits in the `final_edits`'s field. One if the final edit was a cut, two if it was a dissolve or a wipe.

##### Edit Event JSON data
Each event type expects roughly the same JSON payload structure, with optional attribute usage depending on the event or edit type:

```typescript
{
    "edit_type": "cut" | "wipe" | "dissolve",
    "edit_duration_frames"?: number, 
    "wipe_num"?: number,
    "source_tape"?: string,   
    "av_channels"?: {     
        "video": boolean,     
        "audio": number   
    } 
}
```
- `edit_type`: Specifies what the edit type should be - either a cut, a wipe or a dissolve. If the edit type is a dissolve or a wipe, a duration in required in the `edit_duration_frames` field. Wipes can also optionally have a wipe number which can tell the editing system which wipe to use. This is specified in the `wipe_num` field.

- `edit_duration_frames`: Specifies the length of the edit in frames. This value is required for dissolves and wipes. For cuts it is ignored.

- `wipe_num`: Optionally specifies which wipe should be used by the editing system (defaults to `1`). This value is ignored for cuts and dissolves.

- `source_tape`: Optionally specifies the name of the of the tape the edit is being made for. This typically would be the name of the file the source of the video will correspond with in your editing software. The file extension might be needed in such a case depending on the editing software you use. If this filed is not included, EDLgen will attempt to use the preselected source tape which is set by the [**SELECT SOURCE**](#other-events) event.

- `av_channels`: Specifies the video and audio channels.
    - `video`: Specifies if the channel contains video.
    - `audio`: Specifies the number of audio channels.


Examples...
```typescript
// A log event to a preselect source
{   
    "edit_type": "cut",   
    "source_tape": "clip2",   
    "av_channels": {     
        "video": true,     
        "audio": 2   
    } 
}
```
```typescript
// An 18 frame wipe
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
```typescript
// An END event. No source_tape or av_channels is needed as it cuts to black!
{   
    "edit_type": "dissolve",
    "edit_duration_frames": 25,
}
```
#### Other Events

- **SELECT SOURCE** - POST to `127.0.0.1:{port_num}/select-src` - Triggers the pre-selection of a source tape and/or av channels to be used by the next edit events, and will be utilized when an edit event does not contain a `source_tape` or an `av_channels` field.

##### Select Source Event JSON Metadata

```typescript
{
    "source_tape"?: string,
    "av_channels"?: {     
        "video": boolean,     
        "audio": number   
    } 
}
```
- `source_tape`: Specifies the name of the of the tape the next edit is being made for.
- `av_channels`: Specifies the video and audio channels for the next edits.
    - `video`: Specifies if the channel contains video.
    - `audio`: Specifies the number of audio channels.

#### Recording State
Once EDLgen's server is started, it can be in 1 of 3 possible "recording states":

- **stopped** - The server is online but no request has been made to start recording an EDL. 
- **waiting** - A request has been made to start recording an EDL, but an LTC signal has still not received or detected, and the recording process has not begun .
- **started** - The LTC signal has been detected, and the EDL recording has begun.

You can ask EDLgen for what recording state it's currently in:

- GET to `127.0.0.1:{port_num}/edl-recording-state` 

### Planned Features and TODO
- Handle speed changes
- Improved logging 
    - Colors
    - (Dev) Render scroll area with egui rows
    - (Dev) Limit log size?
