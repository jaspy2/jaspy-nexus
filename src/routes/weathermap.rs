extern crate rocket_contrib;
use rocket::{get, put};
use models;
use db;
use rocket_contrib::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use utilities;
use rocket::State;
use std::ops::DerefMut;

fn get_topology_data(connection: &db::Connection) -> models::json::WeathermapBase {
    let mut wmap: models::json::WeathermapBase = models::json::WeathermapBase {
        devices: HashMap::new(),
    };
    let devices = models::dbo::Device::all(&connection);
    for device in devices.iter() {
        let device_fqdn = format!("{}.{}", device.name, device.dns_domain);
        let mut weathermap_device = models::json::WeathermapDevice {
            fqdn: device_fqdn.clone(),
            interfaces: HashMap::new(),
        };

        for interface in device.interfaces(&connection) {
            let connected_interface: Option<models::json::WeathermapDeviceInterfaceConnectedTo> = match interface.peer_interface(&connection) {
                Some(peer_interface) => {
                    let peer_device = peer_interface.device(&connection);
                    let peer_device_fqdn = format!("{}.{}", peer_device.name, peer_device.dns_domain);
                    
                    Some(models::json::WeathermapDeviceInterfaceConnectedTo {
                        fqdn: peer_device_fqdn,
                        interface: peer_interface.name(),
                    })
                },
                None => {
                    None
                }
            };
            let mut weathermap_interface = models::json::WeathermapDeviceInterface {
                name: interface.name(),
                if_index: interface.index,
                connected_to: connected_interface
            };
            weathermap_device.interfaces.insert(
                interface.name(),
                weathermap_interface,
            );
        }
        
        wmap.devices.insert(device_fqdn.clone(), weathermap_device);
    }

    return wmap;
}

#[get("/")]
pub fn full_topology_data(connection: db::Connection, cache_controller: State<Arc<Mutex<utilities::cache::CacheController>>>) -> json::Json<models::json::WeathermapBase> {
    let cached_weathermap_topology_arc: Arc<Mutex<Option<utilities::cache::CachedWeathermapTopology>>>;
    if let Ok(cache_controller) = cache_controller.inner().lock() {
        cached_weathermap_topology_arc = cache_controller.cached_weathermap_topology.clone();
    } else {
        // TODO: log, this means cache is somehow VERY broken
        cached_weathermap_topology_arc = Arc::new(Mutex::new(None));
    }

    let mut ret : models::json::WeathermapBase = models::json::WeathermapBase {
        devices: HashMap::new()
    };

    if let Ok(ref mut cached_weathermap_topology_option_mutex) = cached_weathermap_topology_arc.lock() {
        let mut cached_weathermap_topology_option: &mut Option<utilities::cache::CachedWeathermapTopology> = cached_weathermap_topology_option_mutex.deref_mut();
        let cache_refresh: bool;
        if let Some(cached_weathermap_topology_data) = cached_weathermap_topology_option {
            let current_time = utilities::tools::get_time();
            if current_time < cached_weathermap_topology_data.valid_until {
                ret = cached_weathermap_topology_data.weathermap_topology.clone();
                cache_refresh = false;
            } else {
                ret = get_topology_data(&connection);
                cache_refresh = true;
            }
        } else {
            ret = get_topology_data(&connection);
            cache_refresh = true;
        }
        if cache_refresh {
            *cached_weathermap_topology_option = Some(utilities::cache::CachedWeathermapTopology::new(ret.clone()));
        }
    }
    return json::Json(ret);
}

#[get("/state")]
pub fn state_information(imds: State<Arc<Mutex<utilities::imds::IMDS>>>) -> json::Json<models::json::WeathermapStateBase> {
    let metrics : Option<Vec<models::metrics::LabeledMetric>>;

    if let Ok(ref mut imds) = imds.inner().lock() {
        metrics = Some(imds.get_fast_metrics());
    } else {
        metrics = None;
    }

    let mut weathermap_state = models::json::WeathermapStateBase {
        devices: HashMap::new()
    };
    
    if let Some(metrics) = metrics {
        for metric in metrics.iter() {
            let metric_labels: &HashMap<String,String> = &metric.labels;
            
            if let Some(fqdn) = metric_labels.get("fqdn") {
                if !weathermap_state.devices.contains_key(fqdn) {
                    weathermap_state.devices.insert(fqdn.clone(), models::json::WeathermapStateDevice {
                        state: false,
                        interfaces: HashMap::new()
                    });
                }
                if let Some(device) = weathermap_state.devices.get_mut(fqdn) {
                    if metric.name == "jaspy_device_up" {
                        match metric.value {
                            models::metrics::MetricValue::Int64(v) => {
                                if v == 1 {
                                    device.state = true;
                                } else {
                                    device.state = false;
                                }
                            },
                            models::metrics::MetricValue::Uint64(v) => {
                                if v == 1 {
                                    device.state = true;
                                } else {
                                    device.state = false;
                                }
                            }
                        }
                    } else if metric.name == "jaspy_interface_up" {
                        if let Some(neighbors) = metric_labels.get("neighbors") {
                            if neighbors != "yes" { continue; }
                        }
                        if let Some(interface_name) = metric_labels.get("name") {
                            if device.interfaces.contains_key(interface_name) {
                                // TODO: log, this should never happen!
                                continue;
                            }
                            let state;
                            match metric.value {
                                models::metrics::MetricValue::Int64(v) => {
                                    if v == 1 {
                                        state = true;
                                    } else {
                                        state = false;
                                    }
                                },
                                models::metrics::MetricValue::Uint64(v) => {
                                    if v == 1 {
                                        state = true;
                                    } else {
                                        state = false;
                                    }
                                }
                            }
                            device.interfaces.insert(interface_name.clone(), models::json::WeathermapStateDeviceInterfaceState {
                                state: state,
                            });
                        }
                    }
                }
            }            
        }
    }

    return json::Json(weathermap_state);
}


#[get("/position")]
pub fn get_position_data(connection: db::Connection) -> json::Json<models::json::WeathermapPositionInfoBase> {
    let mut weathermap_position_info = models::json::WeathermapPositionInfoBase {
        devices: HashMap::new(),
    };

    for device in models::dbo::Device::all(&connection) {
        if let Some(wmpi) = device.weathermap_info(&connection) {
            weathermap_position_info.devices.insert(
                format!("{}.{}", device.name, device.dns_domain),
                models::json::WeathermapPositionInfoDeviceInfo {
                    x: wmpi.x,
                    y: wmpi.y,
                    super_node: wmpi.super_node,
                    expanded_by_default: wmpi.expanded_by_default,
                }
            );
        }
    }

    return json::Json(weathermap_position_info);
}

#[put("/position", data = "<device_position_info>")]
pub fn put_position_data(connection: db::Connection, device_position_info : json::Json<models::json::WeathermapPositionInfoUpdateDeviceInfo>) {
    let new_position_info = device_position_info.into_inner();
    if let Ok(_updated_item) = models::dbo::WeathermapDeviceInfo::update_by_fqdn_or_create(
        &connection,
        &new_position_info.device_fqdn,
        models::dbo::UpdatedWeathermapDeviceInfo { 
            x: new_position_info.x,
            y: new_position_info.y,
            expanded_by_default: new_position_info.expanded_by_default,
            super_node: new_position_info.super_node,
        }
    ) {

    }
}
