#![feature(custom_attribute)]
#![feature(plugin)]
#![plugin(rocket_codegen)]
#![allow(proc_macro_derive_resolution_fallback)] // remove when able
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate serde_json;
#[macro_use] extern crate diesel;
extern crate rocket;
extern crate rocket_contrib;
extern crate r2d2;
extern crate r2d2_diesel;
extern crate time;
mod routes;
mod models;
mod db;
mod schema;
mod utilities;
use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicBool;
use std::collections::HashMap;

fn should_continue(running : &std::sync::atomic::AtomicBool) -> bool {
    return running.load(std::sync::atomic::Ordering::Relaxed);
}

fn imds_worker(running : Arc<AtomicBool>, imds : Arc<Mutex<utilities::imds::IMDS>>, metric_miss_cache: Arc<Mutex<models::metrics::DeviceMetricRefreshCacheMiss>>) {
    let mut first_run_done = false;
    let mut refresh_run_counter : i32 = 0;
    let pool = db::connect();
    loop {
        if !should_continue(&running) { break; }
        let mut refresh = false;
        let mut refresh_devices : Vec<models::dbo::Device> = Vec::new();
        let mut refresh_interfaces : HashMap<String, Vec<models::dbo::Interface>> = HashMap::new();
        {
            if let Ok(ref mut metric_miss_cache) = metric_miss_cache.lock() {
                match pool.get() {
                    Ok(conn) => {
                        if !first_run_done || !metric_miss_cache.miss_set.is_empty() {
                            refresh = true;
                            for device in models::dbo::Device::all(&conn).iter() {
                                let device_fqdn = format!("{}.{}", device.name, device.dns_domain);
                                if refresh_run_counter == 0 || metric_miss_cache.miss_set.contains(&device_fqdn) {
                                    refresh_devices.push(device.clone());
                                    refresh_interfaces.insert(device_fqdn, device.interfaces(&conn));
                                }
                            }
                        }
                        metric_miss_cache.miss_set.clear();
                    },
                    Err(_) => {
                        // TODO: log?
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                        continue;
                    }
                }
            }
        }
        {
            if let Ok(ref mut imds) = imds.lock() {
                for device in refresh_devices.iter() {
                    let device_fqdn = format!("{}.{}", device.name, device.dns_domain);
                    imds.refresh_device(&device_fqdn);
                    if let Some(device_interfaces) = refresh_interfaces.get(&device_fqdn) {
                        for interface in device_interfaces.iter() {
                            imds.refresh_interface(&device_fqdn, interface.index, &interface.interface_type, &interface.name(), interface.connected_interface.is_some(), interface.speed_override);
                        }
                    }
                }
                if refresh {
                    imds.prune();
                }
            };
        }
        first_run_done = true;
        if refresh_run_counter >= 9 { refresh_run_counter = 0; } else { refresh_run_counter += 1; }
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}

fn main() {
    let running = Arc::new(AtomicBool::new(true));
    
    let msgbus : Arc<Mutex<utilities::msgbus::MessageBus>> = Arc::new(Mutex::new(utilities::msgbus::MessageBus::new()));
    let imds : Arc<Mutex<utilities::imds::IMDS>> = Arc::new(Mutex::new(utilities::imds::IMDS::new(msgbus.clone())));
    let metric_miss_cache : Arc<Mutex<models::metrics::DeviceMetricRefreshCacheMiss>> = Arc::new(Mutex::new(models::metrics::DeviceMetricRefreshCacheMiss::new()));

    let imds_worker_imds = imds.clone();
    let imds_worker_running = running.clone();
    let imds_worder_metric_miss_cache = metric_miss_cache.clone();
    let imds_worker_thread = std::thread::spawn(|| {
        imds_worker(imds_worker_running, imds_worker_imds, imds_worder_metric_miss_cache);
    });

    let cache_controller : Arc<Mutex<utilities::cache::CacheController>> = Arc::new(Mutex::new(utilities::cache::CacheController::new()));
    
    rocket::ignite()
        .mount(
            "/discovery",
            routes![
                routes::discovery::discovery_device,
                routes::discovery::discovery_links,
            ]
        )
        .mount(
            "/device",
            routes![
                routes::device::device_list,
                routes::device::monitored_device_list,
                routes::device::monitored_device_report,
            ]
        )
        .mount(
            "/interface",
            routes![
                routes::interface::interface_list,
                routes::interface::interface_monitor_report,
            ]
        )
        .mount(
            "/metrics",
            routes![
                routes::metrics::metrics_fast,
                routes::metrics::metrics,
            ]
        )
        .mount(
            "/weathermap",
            routes![
                routes::weathermap::full_topology_data,
                routes::weathermap::state_information,
                routes::weathermap::get_position_data,
                routes::weathermap::put_position_data,
            ]
        )
        .manage(db::connect())
        .manage(imds.clone())
        .manage(metric_miss_cache.clone())
        .manage(cache_controller.clone())
        .launch();

    (*running).store(false, std::sync::atomic::Ordering::Relaxed);
    imds_worker_thread.join().unwrap();
}
