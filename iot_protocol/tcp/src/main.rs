mod tcp_server;

use crate::tcp_server::CLIENTS;
use chrono::Utc;
use log::{debug, info};
use common_lib::protocol_config::read_config;
use common_lib::redis_handler::RedisWrapper;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{interval, Duration};
fn init_logger() {
    log4rs::init_file("log4rs.yml", Default::default()).unwrap();
}

#[tokio::main]
async fn main() {
    init_logger();
    let result = read_config("app-local.yml").unwrap();
    let redis_wrapper = RedisWrapper::new(result.redis_config).unwrap();

    let k1 = format!("tcp_uid_f:{}", result.node_info.name);
    let k2 = format!("tcp_uid:{}", result.node_info.name);
    redis_wrapper.delete_hash(k1.as_str()).await.unwrap();
    redis_wrapper.delete_hash(k2.as_str()).await.unwrap();


    // tokio::spawn(print_clients(CLIENTS,redis_wrapper.clone()));

    let server = crate::tcp_server::TcpServer::new(
        format!("{}:{}", &result.node_info.host, result.node_info.port.to_string().as_str()).as_str(),
        redis_wrapper.clone(),
        result.node_info.name.clone(),
        result.node_info.size,
    );

    // 启动服务器
    tokio::spawn(async move {
        server.start().await;
    });

    tokio::spawn(async {
        print_clients_periodically(redis_wrapper, result.node_info.name).await;
    });

    // 保证 main 不会提前退出
    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl-c signal");
}


pub async fn print_clients_periodically(redis_wrapper: RedisWrapper, name: String) {
    let mut interval = interval(Duration::from_secs(10));

    loop {
        interval.tick().await;

        let mut clients = CLIENTS.lock().await;
        debug!("Current connected clients:");

        let mut disconnected_clients = Vec::new(); // 用于存储需要移除的客户端地址

        for (address, v) in clients.iter() {
            debug!("Client address: {}", address);
            let string = address.replace(":", "@");
            let key = format!("tcp:last:{}", string);

            if let Some(value) = redis_wrapper.get_string(key.as_str()).await.unwrap() {
                match value.parse::<i64>() {
                    Ok(last_active) => {
                        // 获取当前时间
                        let current_time = Utc::now().timestamp(); // 使用 chrono crate 获取 UTC 时间戳
                        let time_difference = current_time - last_active;

                        debug!("Parsed last active time: {}", last_active);
                        debug!("Time difference in seconds: {}", time_difference);

                        // 如果时间差大于 10 秒，关闭 TCP 连接
                        if time_difference > 10 {
                            debug!("Closing TCP connection for: {}", address);
                            disconnected_clients.push(address.clone());

                            // 获取客户端连接并尝试关闭
                            let mut tcp_stream = v.lock().await;
                            if let Err(e) = tcp_stream.shutdown().await {
                                debug!("Failed to close connection for {}: {}", address, e);
                            } else {
                                debug!("Connection closed for: {}", address);
                                // 将需要移除的地址添加到列表
                                cleanup_connection(name.as_str(), address.clone(), redis_wrapper.clone()).await;
                            }
                        }
                    }
                    Err(e) => debug!("Failed to parse number: {}", e),
                }
            }
        }

        // 处理需要移除的客户端
        for address in disconnected_clients {
            clients.remove(&address);
        }

        if clients.is_empty() {
            debug!("No connected clients at this time.");
        }
    }
}
async fn cleanup_connection(name: &str, remote_address: String, redis_wrapper: RedisWrapper) {
    let k1 = format!("tcp_uid:{}", name);
    let k2 = format!("tcp_uid_f:{}", name);
    if let Some(device_id) = redis_wrapper.get_hash(&k2, &remote_address).await.unwrap() {
        redis_wrapper.delete_hash_field(&k1, &device_id).await.expect("Failed to delete hash field");
        redis_wrapper.delete_hash_field(&k2, &remote_address).await.expect("Failed to delete hash field");
    }
}