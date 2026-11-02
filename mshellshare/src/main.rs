#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let payload = std::env::var("XDPH_WINDOW_SHARING_LIST").unwrap_or_default();
    let reply = mshell_ipc::connect().await?.screenshare(&payload).await?;
    println!("{reply}");
    Ok(())
}
