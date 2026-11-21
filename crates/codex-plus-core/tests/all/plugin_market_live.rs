//! 显式运行的联网验证：不修改用户 CODEX_HOME，不调用原生安装器或服务授权。
use codex_plus_core::plugin_market;

#[tokio::test]
#[ignore = "需要访问 GitHub 上的插件缓存仓库"]
async fn index_only_then_install_selected_plugin() {
    let home = tempfile::tempdir().unwrap().keep();
    std::fs::write(home.join("config.toml"), "model = \"test-preserved\"\n").unwrap();
    // 一个 2026-10-08 就有的插件，一个 2026-11-21 新增分组里的插件。
    for (index, (source, name)) in [("full", "life-sciences-databases"), ("full", "1social-scheduler")]
        .into_iter()
        .enumerate()
    {
        let before_packages = std::fs::read_dir(home.join("plugins/cache"))
            .map(|entries| entries.count())
            .unwrap_or_default();
        let before = plugin_market::list_plugins(&home, source, true)
            .await
            .unwrap();
        assert_eq!(before["status"], "ok");
        assert!(!home.join("plugins/cache").exists() || index > 0);
        assert_eq!(
            std::fs::read_dir(home.join("plugins/cache"))
                .map(|entries| entries.count())
                .unwrap_or_default(),
            before_packages
        );
        let entries = before["plugins"].as_array().unwrap();
        let plugin = entries.iter().find(|item| item["name"] == name).unwrap();
        let id = plugin["id"].as_str().unwrap();
        assert_eq!(plugin["installed"], false);
        let installed = plugin_market::install_plugin(&home, source, id)
            .await
            .unwrap();
        assert_eq!(installed["status"], "ok");
        assert_eq!(installed["plugin"]["installed"], true);
        let status = plugin_market::install_status(&home, source, id).unwrap();
        assert_eq!(status["stage"], "complete");
        assert_eq!(status["busy"], false);
        let after = plugin_market::list_plugins(&home, source, false)
            .await
            .unwrap();
        let installed_count = after["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["installed"] == true)
            .count();
        assert_eq!(installed_count, index + 1, "只安装当前选中的插件");
        assert!(
            std::fs::read_to_string(home.join("config.toml"))
                .unwrap()
                .contains("test-preserved")
        );
        let native =
            plugin_market::native_request(&home, "installed-plugins", serde_json::json!({}))
                .await
                .unwrap();
        assert!(
            native["marketplaces"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|market| market["plugins"].as_array().unwrap())
                .any(|item| item["id"]
                    == installed["plugin"]["marketplaceName"]
                        .as_str()
                        .map(|market| format!("{name}@{market}"))
                        .map(serde_json::Value::String)
                        .unwrap())
        );
        println!(
            "source={source} catalog={} installed={name} marketplace={}",
            entries.len(),
            installed["plugin"]["marketplaceName"]
        );
    }
    println!("isolated_home={}", home.display());
}
