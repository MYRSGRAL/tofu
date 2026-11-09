//! Локальная синтетическая нагрузка: сеть, keyring и пользовательская БД не используются.

#[cfg(target_os = "linux")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        fmt::Write,
        fs::{self, DirBuilder},
        os::unix::fs::DirBuilderExt,
        time::Instant,
    };
    use tofu_linux::{EncryptedSubscriptionRepository, ProfileQuery, ProfileSort, SnapshotInput};
    use zeroize::Zeroizing;
    let count: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "10000".into())
        .parse()?;
    if !(1..=20_000).contains(&count) {
        return Err("Число записей должно быть от 1 до 20000".into());
    }
    let root = std::env::temp_dir().join(format!("tofu-profile-bench-{}", std::process::id()));
    DirBuilder::new().mode(0o700).create(&root)?;
    let result=async {
        let started=Instant::now();
        let repo=EncryptedSubscriptionRepository::open(root.join("profiles.sqlite"),Zeroizing::new([17;32])).await?;
        let source=repo.create_source(b"synthetic-only".to_vec()).await?;
        let mut body=String::with_capacity(count*110);
        for i in 0..count { writeln!(body,"trojan://invented-password-{i}@server-{i}.example.invalid:443#Сервер {i:05}")?; }
        let commit_started=Instant::now();
        repo.commit_snapshot(source.id,1,0,SnapshotInput {body:body.as_bytes().to_vec(),metadata:vec![],validators:None}).await?;
        let commit_ms=commit_started.elapsed().as_millis();
        let list_started=Instant::now();
        let page=repo.profiles(source.id,ProfileQuery{sort:ProfileSort::Name,..Default::default()}).await?;
        let list_ms=list_started.elapsed().as_millis();assert_eq!(page.total,count);
        let favorite_started=Instant::now();
        repo.set_favorite(page.profiles[0].id,true).await?;
        let favorite_ms=favorite_started.elapsed().as_millis();
        let search_started=Instant::now();
        let found=repo.profiles(source.id,ProfileQuery{search:"server-0.example.invalid".into(),..Default::default()}).await?;
        let search_ms=search_started.elapsed().as_millis();assert_eq!(found.total,1);
        let resolve_started=Instant::now();
        let selected=repo.resolve_profile(found.profiles[0].id,1).await?;
        let resolve_ms=resolve_started.elapsed().as_millis();assert!(selected.profile.favorite);
        repo.close().await?;
        let bytes=fs::read_dir(&root)?.try_fold(0_u64,|sum,entry|->std::io::Result<u64>{Ok(sum+entry?.metadata()?.len())})?;
        println!("{}",serde_json::json!({"records":count,"commit_ms":commit_ms,"list_ms":list_ms,"favorite_ms":favorite_ms,"search_ms":search_ms,"resolve_ms":resolve_ms,"total_ms":started.elapsed().as_millis(),"database_bytes":bytes}));
        Ok::<(),Box<dyn std::error::Error>>(())
    }.await;
    fs::remove_dir_all(&root)?;
    result
}

#[cfg(not(target_os = "linux"))]
fn main() {}
