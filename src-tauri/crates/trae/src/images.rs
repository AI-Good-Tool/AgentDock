//! Official resource upload → Remote `image_id` bridge. The wire format follows
//! Trae SOLO's ImageUpload SDK (`get_resource_upload_url`, PUT with CRC32,
//! `commit_resource_upload_result`). Image bytes never become prompt text.
//! Protocol reference: https://solo.trae.cn/ (ImageUpload SDK, 2026-10-22).
use agentdock_core::provider::SendFailure;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MIME_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && a != 0
                && a < 240
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (18..=19).contains(&b))
                && !(a == 192 && b == 0 && c == 0)
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(ip));
            }
            let segments = ip.segments();
            segments[0] & 0xe000 == 0x2000
                && !(segments[0] == 0x2001 && (segments[1] < 0x200 || segments[1] == 0xdb8))
                && !(segments[0] == 0x3fff && segments[1] < 0x1000)
        }
    }
}

fn invalid(message: &str) -> SendFailure {
    SendFailure::local(400, "invalid_request", message)
}

fn download_failed(message: &str) -> SendFailure {
    SendFailure::local(502, "image_download_failed", message)
}

pub fn validate_url(url: &str) -> Result<(), &'static str> {
    if let Some(data) = url.strip_prefix("data:") {
        let (mime, encoded) = data
            .split_once(";base64,")
            .ok_or("Trae 图片必须使用 base64 数据 URL")?;
        if !MIME_TYPES.contains(&mime) {
            return Err("Trae 不支持此图片格式");
        }
        if encoded.is_empty() || encoded.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
            return Err("Trae 图片为空或超过 20 MB 限制");
        }
    } else {
        let parsed = reqwest::Url::parse(url).map_err(|_| "Trae 图片 URL 无效")?;
        if parsed.scheme() != "https"
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err("Trae 图片 URL 必须使用 HTTPS 且不能包含凭证");
        }
        let host = parsed.host_str().unwrap().trim_matches(['[', ']']);
        if host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
            || host.parse::<IpAddr>().is_ok_and(|ip| !public_ip(ip))
        {
            return Err("Trae 图片 URL 不允许访问本机或私网地址");
        }
    }
    Ok(())
}

async fn download_client(url: &str) -> Result<reqwest::Client, SendFailure> {
    let parsed = reqwest::Url::parse(url).map_err(|_| invalid("Trae 图片 URL 无效"))?;
    let host = parsed.host_str().unwrap().trim_matches(['[', ']']);
    let port = parsed.port_or_known_default().unwrap();
    let addresses: Vec<SocketAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::time::timeout(
            Duration::from_secs(15),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| download_failed("Trae 图片域名解析超时"))?
        .map_err(|_| download_failed("Trae 图片域名解析失败"))?
        .collect(),
    };
    if addresses.is_empty() || addresses.iter().any(|address| !public_ip(address.ip())) {
        return Err(invalid("Trae 图片 URL 不允许访问本机或私网地址"));
    }
    // Pin the checked DNS answers and bypass ambient proxies, so the actual
    // connection cannot resolve the same hostname to an unchecked private IP.
    reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| download_failed("Trae 图片下载客户端初始化失败"))
}

async fn load(url: &str) -> Result<(String, Vec<u8>), SendFailure> {
    validate_url(url).map_err(invalid)?;
    let (mime, bytes) = if let Some(data) = url.strip_prefix("data:") {
        let (mime, encoded) = data.split_once(";base64,").unwrap();
        (
            mime.to_owned(),
            B64.decode(encoded)
                .map_err(|_| invalid("Trae 图片 base64 数据无效"))?,
        )
    } else {
        // No account credentials are sent to the caller's image host.
        let response = download_client(url)
            .await?
            .get(url)
            .send()
            .await
            .map_err(|_| download_failed("Trae 图片下载失败"))?;
        if !response.status().is_success() {
            return Err(download_failed(&format!(
                "Trae 图片下载失败（HTTP {}）",
                response.status().as_u16()
            )));
        }
        let mime = response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_owned();
        if !MIME_TYPES.contains(&mime.as_str()) {
            return Err(invalid("Trae 图片 URL 未返回支持的图片格式"));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_IMAGE_BYTES as u64)
        {
            return Err(invalid("Trae 图片超过 20 MB 限制"));
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| download_failed("Trae 图片下载中断"))?;
            if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
                return Err(invalid("Trae 图片超过 20 MB 限制"));
            }
            bytes.extend_from_slice(&chunk);
        }
        (mime, bytes)
    };
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(invalid("Trae 图片为空或超过 20 MB 限制"));
    }
    Ok((mime, bytes))
}

#[derive(Debug)]
pub struct PreparedImage {
    mime: String,
    bytes: Vec<u8>,
    width: usize,
    height: usize,
}

pub async fn prepare(url: &str) -> Result<PreparedImage, SendFailure> {
    let (mime, bytes) = load(url).await?;
    let actual_mime =
        match imagesize::image_type(&bytes).map_err(|_| invalid("Trae 图片数据无效"))? {
            imagesize::ImageType::Png => "image/png",
            imagesize::ImageType::Jpeg => "image/jpeg",
            imagesize::ImageType::Gif => "image/gif",
            imagesize::ImageType::Webp => "image/webp",
            _ => return Err(invalid("Trae 不支持此图片格式")),
        };
    if actual_mime != mime {
        return Err(invalid("Trae 图片格式与声明的 MIME 类型不一致"));
    }
    let size =
        imagesize::blob_size(&bytes).map_err(|_| invalid("Trae 图片数据无效或无法读取尺寸"))?;
    if size.width < 14 || size.height < 14 || size.width.saturating_mul(size.height) > 36_000_000 {
        return Err(invalid("Trae 图片尺寸须至少 14×14，且不超过 3600 万像素"));
    }
    Ok(PreparedImage {
        mime,
        bytes,
        width: size.width,
        height: size.height,
    })
}

fn checked_data(status: reqwest::StatusCode, data: Value, route: &str) -> Result<Value, String> {
    let success = matches!(&data["code"], Value::Null)
        || data["code"] == 0
        || data["code"] == "0"
        || data["code"] == "";
    if !status.is_success() || !success {
        return Err(format!(
            "Trae 图片上传接口 {route} 失败（HTTP {}）",
            status.as_u16()
        ));
    }
    Ok(if data["data"].is_object() {
        data["data"].clone()
    } else {
        data
    })
}

async fn post(account: &Value, host: &str, route: &str, body: Value) -> Result<Value, String> {
    let mut req = crate::api::http()
        .post(format!("{host}/api/ide/v1/{route}"))
        .json(&body);
    for (key, value) in super::gateway::remote_headers(account) {
        req = req.header(key, value);
    }
    let response = req.send().await.map_err(|_| "Trae 图片上传接口连接失败")?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Trae 图片上传接口 {route} 失败（HTTP {}）",
            status.as_u16()
        ));
    }
    let data: Value = response
        .json()
        .await
        .map_err(|_| "Trae 图片上传接口响应不是 JSON")?;
    checked_data(status, data, route)
}

pub async fn upload(
    account: &Value,
    host: &str,
    image: PreparedImage,
    fixture: bool,
) -> Result<String, String> {
    let PreparedImage {
        mime,
        bytes,
        width,
        height,
    } = image;
    let target = format!(
        "{}_{}x{}.trae",
        uuid::Uuid::new_v4().simple(),
        width,
        height
    );
    let auth = post(
        account,
        host,
        "get_resource_upload_url",
        json!({
            "targets":[target], "biz_type":"image",
            "scale_param":{"width":width,"height":height},
        }),
    )
    .await?;
    let upload_host = auth["upload_hosts"][0]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Trae 图片上传地址缺失")?;
    let info = &auth["store_infos"][0];
    let uri = info["store_uri"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Trae 图片上传资源 ID 缺失")?;
    let origin = if upload_host.contains("://") {
        upload_host.to_owned()
    } else {
        format!("https://{upload_host}")
    };
    let destination = reqwest::Url::parse(&format!(
        "{}/{}",
        origin.trim_end_matches('/'),
        uri.trim_start_matches('/')
    ))
    .map_err(|_| "Trae 图片上传地址无效")?;
    if destination.scheme() != "https"
        && !(fixture && destination.origin().ascii_serialization() == host)
    {
        return Err("Trae 图片上传地址必须使用 HTTPS".into());
    }
    let mut req = crate::api::http()
        .put(destination)
        .header("Content-Type", mime)
        .header("Content-CRC32", format!("{:08x}", crc32fast::hash(&bytes)));
    // Only the scoped upload authorization goes to the storage host.
    if let Some(token) = info["auth"].as_str().filter(|s| !s.is_empty()) {
        req = req.header("Authorization", token);
    }
    let response = req
        .body(bytes)
        .send()
        .await
        .map_err(|_| "Trae 图片上传失败")?;
    if !response.status().is_success() {
        return Err(format!(
            "Trae 图片上传失败（HTTP {}）",
            response.status().as_u16()
        ));
    }
    if let Some(session) = auth["session_key"].as_str().filter(|s| !s.is_empty()) {
        post(
            account,
            host,
            "commit_resource_upload_result",
            json!({
                "oids":[uri], "success_oids":[uri], "session_key":session, "biz_type":"image",
            }),
        )
        .await?;
    }
    Ok(info["override_resource_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(uri)
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_sources_are_bounded_and_explicit() {
        for url in [
            "file:///tmp/a.png",
            "http://example.com/a.png",
            "https://user:secret@example.com/a.png",
            "data:text/plain;base64,YQ==",
            "data:image/png,raw",
            "data:image/png;base64,",
            "https://localhost/image.png",
            "https://127.0.0.1/image.png",
            "https://10.0.0.1/image.png",
            "https://[::1]/image.png",
            "https://[::ffff:127.0.0.1]/image.png",
        ] {
            assert!(validate_url(url).is_err(), "{url}");
        }
        assert!(validate_url("https://example.com/a.png").is_ok());
        assert!(validate_url("data:image/png;base64,YQ==").is_ok());
    }

    #[tokio::test]
    async fn invalid_base64_is_rejected() {
        let error = load("data:image/png;base64,!!!!").await.unwrap_err();
        assert_eq!(error.status, 400);
        assert!(error.message.contains("base64"));
    }

    #[test]
    fn only_public_download_addresses_are_accepted() {
        for address in [
            "0.0.0.0",
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "198.18.0.1",
            "224.0.0.1",
            "240.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:192.168.1.1",
            "2001:db8::1",
        ] {
            assert!(!public_ip(address.parse().unwrap()), "{address}");
        }
        for address in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public_ip(address.parse().unwrap()), "{address}");
        }
    }

    #[tokio::test]
    async fn private_dns_answers_are_rejected_before_connecting() {
        let error = download_client("https://localhost/image.png")
            .await
            .unwrap_err();
        assert_eq!(error.status, 400);
        assert!(error.message.contains("私网"));
    }

    #[tokio::test]
    async fn invalid_image_bytes_and_mismatched_mime_are_client_errors() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAF0lEQVR4nGP4z8BAEiJN9aiGUQ1DSgMAkPn/Afnh+ngAAAAASUVORK5CYII=";
        assert!(prepare(&format!("data:image/png;base64,{png}"))
            .await
            .is_ok());
        for source in [
            "data:image/png;base64,YQ==".to_owned(),
            format!("data:image/jpeg;base64,{png}"),
        ] {
            assert_eq!(prepare(&source).await.unwrap_err().status, 400);
        }
    }

    #[test]
    fn upload_business_errors_cannot_be_treated_as_successful_commits() {
        for code in [json!(403), json!("403"), json!(false)] {
            assert!(checked_data(reqwest::StatusCode::OK, json!({"code":code}), "commit").is_err());
        }
        for code in [json!(0), json!("0"), Value::Null] {
            assert_eq!(
                checked_data(
                    reqwest::StatusCode::OK,
                    json!({"code":code,"data":{"ok":true}}),
                    "commit"
                )
                .unwrap(),
                json!({"ok":true})
            );
        }
    }
}
