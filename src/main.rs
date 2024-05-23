use rusb::{
    self, Context, Device, DeviceDescriptor, DeviceHandle, Direction, TransferType, UsbContext,
};
#[allow(unused_imports)]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
#[allow(unused_imports)]
use std::thread;
use std::time::Duration;

#[allow(dead_code)]
#[derive(Debug)]
struct Endpoint {
    config: u8,
    iface: u8,
    setting: u8,
    address: u8,
}

impl Endpoint {
    fn new(config: u8, iface: u8, setting: u8, address: u8) -> Self {
        Endpoint {
            config,
            iface,
            setting,
            address,
        }
    }
}

fn main() -> rusb::Result<()> {
    let mut context = Arc::new(Context::new()?);
    let mut handle = capture_key_press(&mut context)?;

    print_device_info(&mut handle)?;
    Ok(())
}

fn capture_key_press<T: UsbContext>(context: &mut Arc<T>) -> rusb::Result<DeviceHandle<T>> {
    let devices = context.devices()?;
    for mut device in devices.iter() {
        println!("device: {:#?}", device);

        let endpoints = find_readable_endpoints(&mut device, TransferType::Interrupt)?;
        println!("endpoints: {:#?}", endpoints);

        match device.open() {
            Ok(mut handle) => loop {
                let endpoint = endpoints.first().unwrap();

                let has_kernel_driver = match handle.kernel_driver_active(endpoint.iface) {
                    Ok(true) => {
                        handle.detach_kernel_driver(endpoint.iface)?;
                        true
                    }
                    _ => false,
                };

                configure_endpoint(&mut handle, endpoint)?;

                let mut buf = [0u8; 64];
                let timeout = Duration::from_secs(1);
                let interrupt = handle.read_interrupt(endpoint.address, &mut buf, timeout);

                if has_kernel_driver {
                    handle.attach_kernel_driver(endpoint.iface).ok();
                }

                match interrupt {
                    Ok(_) => {
                        println!("read Interrupt: {:#?}", buf);
                        return Ok(handle);
                    }
                    Err(rusb::Error::Timeout) => continue,
                    Err(e) => {
                        eprint!("could not read Interrupt {:?}", e);
                        return Err(e);
                    }
                };
            },
            e @ Err(_) => {
                eprint!("could not open device {:?}", e);
                return e;
            }
        }
    }
    Err(rusb::Error::NoDevice)
}

// returns all readable endpoints for given usb device and descriptor
fn find_readable_endpoints<T: UsbContext>(
    device: &mut Device<T>,
    transfer_type: TransferType,
) -> rusb::Result<Vec<Endpoint>> {
    let device_desc = device.device_descriptor()?;
    let mut endpoints = vec![];
    for n in 0..device_desc.num_configurations() {
        let config_desc = match device.config_descriptor(n) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for interface in config_desc.interfaces() {
            for interface_desc in interface.descriptors() {
                for endpoint_desc in interface_desc.endpoint_descriptors() {
                    match endpoint_desc.direction() {
                        Direction::In if endpoint_desc.transfer_type() == transfer_type => {
                            endpoints.push(Endpoint::new(
                                config_desc.number(),
                                interface_desc.interface_number(),
                                interface_desc.setting_number(),
                                endpoint_desc.address(),
                            ));
                        }
                        _ => continue,
                    }
                }
            }
        }
    }

    Ok(endpoints)
}

fn configure_endpoint<T: UsbContext>(
    handle: &mut DeviceHandle<T>,
    endpoint: &Endpoint,
) -> rusb::Result<()> {
    handle.set_active_configuration(endpoint.config)?;
    handle.claim_interface(endpoint.iface)?;
    handle.set_alternate_setting(endpoint.iface, endpoint.setting)?;
    Ok(())
}

#[allow(dead_code)]
fn open_device<T: UsbContext>(
    context: &mut T,
    vid: u16,
    pid: u16,
) -> Option<(Device<T>, DeviceDescriptor, DeviceHandle<T>)> {
    let devices = match context.devices() {
        Ok(d) => d,
        Err(_) => return None,
    };

    for device in devices.iter() {
        let device_desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };

        if device_desc.vendor_id() == vid && device_desc.product_id() == pid {
            match device.open() {
                Ok(handle) => return Some((device, device_desc, handle)),
                Err(e) => panic!("Device found but failed to open: {}", e),
            }
        }
    }
    None
}

fn print_device_info<T: UsbContext>(handle: &mut DeviceHandle<T>) -> rusb::Result<()> {
    let device_desc = handle.device().device_descriptor()?;
    let timeout = Duration::from_secs(1);
    let languages = handle.read_languages(timeout)?;

    println!("Active configuration: {}", handle.active_configuration()?);

    if !languages.is_empty() {
        let language = languages[0];
        println!("Language: {:?}", language);

        println!(
            "Manufacturer: {}",
            handle
                .read_manufacturer_string(language, &device_desc, timeout)
                .unwrap_or("Not Found".to_string())
        );
        println!(
            "Product: {}",
            handle
                .read_product_string(language, &device_desc, timeout)
                .unwrap_or("Not Found".to_string())
        );
        println!(
            "Serial Number: {}",
            handle
                .read_serial_number_string(language, &device_desc, timeout)
                .unwrap_or("Not Found".to_string())
        );
    }
    Ok(())
}
//
// use hidapi::HidApi;
// use std::sync::atomic::{AtomicBool, Ordering};
// use std::sync::{mpsc, Arc, Mutex};
// use std::thread;

// fn main() {
//     let hid = Arc::new(Mutex::new(HidApi::new().unwrap()));
//     let devices = init_usb_devices(&(hid.lock().unwrap()));
//     capture_key_press(hid, devices).unwrap();
// }

// fn init_usb_devices(hid: &HidApi) -> Vec<(u16, u16, String)> {
//     let mut devs: Vec<(u16, u16, String)> = hid
//         .device_list()
//         .map(|device| {
//             let vid = device.vendor_id();
//             let pid = device.product_id();
//             let name = device.product_string().unwrap().to_owned();
//             (vid, pid, name)
//         })
//         .collect();

//     devs.sort();
//     devs.dedup();
//     devs.iter().for_each(|(vid, pid, name)| {
//         println!("VID: {:04x}, PID: {:04x} Prod Name: {:?}", vid, pid, name)
//     });
//     devs
// }

// fn capture_key_press(
//     api: Arc<Mutex<HidApi>>,
//     devices: Vec<(u16, u16, String)>,
// ) -> Result<(), Box<dyn std::error::Error>> {
//     let mut handles = vec![];
//     let is_done = Arc::new(AtomicBool::new(false));

//     let (sender, receiver) = mpsc::channel();
//     for (vid, pid, name) in devices {
//         let api = Arc::clone(&api);
//         let sender = sender.clone();
//         let is_done = Arc::clone(&is_done);
//         let handle = thread::spawn(move || {
//             let device = match api.lock().unwrap().open(vid, pid) {
//                 Ok(device) => {
//                     println!(
//                         "Opened device VID: {:04x}, PID: {:04x}, Name: {}",
//                         vid, pid, name
//                     );
//                     device
//                 }
//                 Err(e) => {
//                     eprintln!(
//                         "Failed to open device VID: {:04x}, PID: {:04x}, Name: {} - {}",
//                         vid, pid, name, e
//                     );
//                     return;
//                 }
//             };

//             device.set_blocking_mode(false).unwrap();

//             let mut buf = [0u8; 256];
//             while !is_done.load(Ordering::Relaxed) {
//                 match device.read(&mut buf) {
//                     Ok(len) => {
//                         if len > 0 {
//                             println!(
//                                 "Device VID: {:04x}, PID: {:04x}, Name: {} - Key Press Detected: {:?}",
//                                 vid,
//                                 pid,
//                                 name,
//                                 &buf[..len],
//                             );
//                             sender.send((vid, pid)).unwrap();
//                             return;
//                         }
//                     }
//                     Err(e) => {
//                         eprintln!(
//                             "Error reading from device VID: {:04x}, PID: {:04x} - {}",
//                             vid, pid, e
//                         );
//                         break;
//                     }
//                 }
//             }
//         });
//         handles.push(handle);
//     }

//     match receiver.recv() {
//         Ok((vid, pid)) => {
//             println!("channel recived VID: {:04x} and PID: {:04x}", vid, pid);
//             is_done.store(true, Ordering::Relaxed);
//         }
//         Err(err) => {
//             eprintln!("{err}");
//         }
//     }

//     for handle in handles {
//         println!("handle joined: {:?}", handle);
//         handle.join().unwrap();
//     }

//     Ok(())
// }
