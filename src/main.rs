use make87::{get_provider, resolve_endpoint_name};
use make87_messages::spatial::translation::{Translation1D, Translation2D};
use make87_messages::well_known_types::Timestamp;
use make87_messages::CurrentTime;
use ros2_client::ros2::{policy, QosPolicyBuilder};
use ros2_client::{Context, Name, NodeName, NodeOptions, ServiceMapping, ServiceTypeName};
use ros2_interfaces_rolling::example_interfaces::srv::{AddTwoInts, AddTwoIntsRequest};
use std::sync::Arc;
use tokio::runtime::Handle;
use uuid::Uuid;

fn sanitize_and_checksum(input: &str) -> String {
    let prefix = "ros2_";

    // Sanitize the input string
    let mut sanitized = String::with_capacity(input.len());
    for c in input.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            sanitized.push(c);
        } else {
            sanitized.push('_');
        }
    }

    // Compute checksum
    let mut sum: u64 = 0;
    for b in input.bytes() {
        sum = (sum * 31 + b as u64) % 1_000_000_007;
    }
    let checksum = sum.to_string();

    // Calculate maximum allowed length for the sanitized string
    const MAX_TOTAL_LENGTH: usize = 256;
    let prefix_length = prefix.len();
    let checksum_length = checksum.len();
    let max_sanitized_length = MAX_TOTAL_LENGTH - prefix_length - checksum_length;

    // Truncate sanitized string if necessary
    if sanitized.len() > max_sanitized_length {
        sanitized.truncate(max_sanitized_length);
    }

    // Construct the final string
    format!("{}{}{}", prefix, sanitized, checksum)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    make87::initialize();

    // Create ROS2 node and client
    let context = Context::new()?;
    let node_id = format!("make87_{}", Uuid::new_v4().simple());

    let mut node = context.new_node(NodeName::new("/make87", &node_id)?, NodeOptions::new())?;

    let service_qos = QosPolicyBuilder::new()
        .reliability(policy::Reliability::Reliable {
            max_blocking_time: ros2_client::ros2::Duration::from_millis(100),
        })
        .history(policy::History::KeepLast { depth: 1 })
        .build();

    let ros_client_name = resolve_endpoint_name("REQUESTER_ENDPOINT")
        .map(|name| sanitize_and_checksum(&name)) // Prefix and replace '.' with '_'
        .ok_or_else(|| "Failed to resolve topic name REQUESTER_ENDPOINT")?;

    // Wrap the client in Arc and Mutex to allow sharing and mutation across threads
    let proxy_ros_client = Arc::new(node.create_client::<AddTwoInts>(
        ServiceMapping::Enhanced,
        &Name::new("/", &ros_client_name)?,
        &ServiceTypeName::new("example_interfaces", "AddTwoInts"),
        service_qos.clone(),
        service_qos,
    )?);

    // Run background tasks
    Handle::current().spawn(node.spinner()?.spin());

    // Now set up the provider
    let make87_endpoint_name = resolve_endpoint_name("PROVIDER_ENDPOINT")
        .ok_or_else(|| "Failed to resolve topic name PROVIDER_ENDPOINT")?;

    let proxy_make87_provider = Arc::new(
        get_provider::<Translation2D, Translation1D>(make87_endpoint_name)
            .ok_or_else(|| "Failed to get provider for PROVIDER_ENDPOINT")?,
    );

    // Provide the service
    proxy_make87_provider
        .provide_async(move |req: Translation2D| {
            let client = Arc::clone(&proxy_ros_client);

            async move {
                println!("Received request: {:?}", req);

                // Lock the client and extract necessary data
                let ros_request = {
                    // If you need to clone any data from client, do it here
                    AddTwoIntsRequest {
                        a: req.x as i64,
                        b: req.y as i64,
                    }
                }; // MutexGuard is dropped here

                // Now perform async operations without holding the MutexGuard
                match client.async_send_request(ros_request).await {
                    Ok(req_id) => {
                        // Wait for the response
                        let response = client.async_receive_response(req_id).await;

                        match response {
                            Ok(response) => {
                                println!("<<< response: {:?}", response);

                                // Wrap the response into a struct called `Translation1D`
                                Translation1D {
                                    timestamp: Timestamp::get_current_time(),
                                    x: response.sum as f32,
                                }
                            }
                            Err(e) => {
                                println!("<<< response error {:?}", e);
                                Translation1D {
                                    timestamp: None,
                                    x: 0.0,
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!(">>> request sending error {:?}", e);
                        Translation1D {
                            timestamp: None,
                            x: 0.0,
                        }
                    }
                }
            }
        })
        .await
        .unwrap();

    make87::keep_running();

    Ok(())
}
