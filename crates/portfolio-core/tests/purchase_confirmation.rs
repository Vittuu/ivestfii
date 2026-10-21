use chrono::NaiveDate;
use ivestfii_core::{
    FundInput, PurchaseConfirmationInput, PurchaseConfirmationItem, RecordInput, Repository,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::sync::{Arc, Barrier};
use tempfile::TempDir;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 21).unwrap()
}
fn setup() -> (TempDir, Repository, Vec<String>) {
    let directory = TempDir::new().unwrap();
    let repo = Repository::open(directory.path().join("portfolio.sqlite")).unwrap();
    for ticker in ["ONE11", "TWO11"] {
        repo.upsert_fund(FundInput {
            id: None,
            ticker: ticker.into(),
            name: ticker.into(),
            sector: "Papel".into(),
        })
        .unwrap();
    }
    let ids = repo
        .snapshot()
        .unwrap()
        .funds
        .into_iter()
        .map(|fund| fund.id)
        .collect();
    (directory, repo, ids)
}
fn purchase(fund_id: &str, shares: &str, price: &str) -> PurchaseConfirmationItem {
    PurchaseConfirmationItem {
        fund_id: fund_id.into(),
        purchase_date: "2026-10-20".into(),
        shares: shares.into(),
        price_per_share: price.into(),
    }
}
fn input(items: Vec<PurchaseConfirmationItem>) -> PurchaseConfirmationInput {
    PurchaseConfirmationInput {
        source_tool_id: "real-saved-tool-id".into(),
        purchases: items,
    }
}
fn record(fund_id: &str, date: Option<&str>, shares: &str, price: &str) -> RecordInput {
    RecordInput {
        id: None,
        fund_id: fund_id.into(),
        month: "2026-10".into(),
        purchase_date: date.map(str::to_owned),
        cotas_added: shares.into(),
        price_per_cota: price.into(),
        dividend_per_cota: "0".into(),
        dividend_total: None,
        notes: "original".into(),
    }
}
fn state(directory: &TempDir, repo: &Repository) -> Value {
    let db = Connection::open(directory.path().join("portfolio.sqlite")).unwrap();
    let receipts: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM simulation_purchase_receipts",
            [],
            |row| row.get(0),
        )
        .unwrap();
    json!({"snapshot":repo.snapshot().unwrap(),"outbox":repo.pending_changes().unwrap(),"export":repo.export_legacy_json().unwrap(),"receipts":receipts})
}

#[test]
fn preview_is_read_only_and_confirmation_is_exact_pure_purchase_batch() {
    let (directory, repo, ids) = setup();
    let before = state(&directory, &repo);
    let request = input(vec![
        purchase(&ids[0], "17", "9.46"),
        purchase(&ids[1], "6", "20.00"),
    ]);
    let preview = repo
        .preview_simulation_purchases(request.clone(), &ids, today())
        .unwrap();
    assert!(preview.can_confirm);
    assert_eq!(
        preview
            .rows
            .iter()
            .map(|row| row.total.as_str())
            .collect::<Vec<_>>(),
        vec!["160.82", "120"]
    );
    assert_eq!(state(&directory, &repo), before);
    let confirmed = repo
        .confirm_simulation_purchases(request, &ids, today(), &preview.preview_token)
        .unwrap();
    assert_eq!(confirmed.created_count, 2);
    assert_eq!(confirmed.matched_count, 0);
    assert!(!confirmed.can_confirm);
    assert!(confirmed
        .rows
        .iter()
        .all(|row| row.status == "already_confirmed"));
    let snapshot = repo.snapshot().unwrap();
    assert_eq!(snapshot.record_count, 2);
    assert_eq!(snapshot.totals.invested, "280.82");
    assert_eq!(snapshot.totals.cotas, "23");
    assert_eq!(snapshot.totals.dividends_received, "0");
    for fund in snapshot.funds {
        let entry = &fund.entries[0];
        assert_eq!(entry.month, "2026-10");
        assert_eq!(entry.purchase_date.as_deref(), Some("2026-10-20"));
        assert_eq!(entry.dividend_total, None);
        assert_eq!(entry.dividend_per_cota, "0");
    }
    let changes = repo.pending_changes().unwrap();
    assert_eq!(changes.len(), 4);
    assert_eq!(
        changes
            .iter()
            .filter(|row| row.entity_type == "monthlyRecord")
            .count(),
        2
    );
    assert_eq!(state(&directory, &repo)["receipts"], 2);
}

#[test]
fn repeated_confirmation_survives_reopen_without_touching_records_or_outbox() {
    let (directory, repo, ids) = setup();
    let request = input(vec![purchase(&ids[0], "17", "9.46")]);
    let preview = repo
        .preview_simulation_purchases(request.clone(), &ids, today())
        .unwrap();
    repo.confirm_simulation_purchases(request.clone(), &ids, today(), &preview.preview_token)
        .unwrap();
    let before = state(&directory, &repo);
    drop(repo);
    let repo = Repository::open_existing(directory.path().join("portfolio.sqlite")).unwrap();
    let replay = repo
        .confirm_simulation_purchases(request, &ids, today(), "old-lost-response-token")
        .unwrap();
    assert_eq!(replay.created_count, 0);
    assert_eq!(replay.matched_count, 1);
    assert_eq!(replay.rows[0].status, "already_confirmed");
    assert!(!replay.can_confirm);
    assert_eq!(state(&directory, &repo), before);
    let restored = repo
        .simulation_purchase_confirmations("real-saved-tool-id", &ids)
        .unwrap();
    assert_eq!(restored.rows, replay.rows);
    assert!(!restored.can_confirm);
    assert_eq!(state(&directory, &repo), before);
    assert!(repo
        .simulation_purchase_confirmations("real-saved-tool-id", &[])
        .is_err());
    assert!(repo
        .simulation_purchase_confirmations("other-tool-id", &ids)
        .unwrap()
        .rows
        .is_empty());
}

#[test]
fn unique_economic_match_links_existing_mixed_record_without_changing_dividends() {
    let (directory, repo, ids) = setup();
    let mut existing = record(&ids[0], Some("2026-10-20"), "17.00", "9.4600");
    existing.dividend_total = Some("1.73".into());
    existing.dividend_per_cota = "0.101".into();
    existing.notes = "Manter comprovante e recebimento".into();
    repo.save_monthly_record(existing).unwrap();
    let before = state(&directory, &repo);
    let request = input(vec![purchase(&ids[0], "017", "9.46")]);
    let preview = repo
        .preview_simulation_purchases(request.clone(), &ids, today())
        .unwrap();
    assert!(preview.can_confirm);
    assert_eq!(preview.rows[0].status, "already_saved");
    let result = repo
        .confirm_simulation_purchases(request, &ids, today(), &preview.preview_token)
        .unwrap();
    assert_eq!(result.created_count, 0);
    assert_eq!(result.matched_count, 1);
    let after = state(&directory, &repo);
    assert_eq!(after["snapshot"], before["snapshot"]);
    assert_eq!(after["export"], before["export"]);
    assert_eq!(after["outbox"], before["outbox"]);
    assert_eq!(after["receipts"], 1);
}

#[test]
fn duplicates_and_undated_legacy_matches_block_entire_batch_without_guessing() {
    for date in [None, Some("2026-10-20")] {
        let (directory, repo, ids) = setup();
        repo.save_monthly_record(record(&ids[0], date, "17", "9.46"))
            .unwrap();
        if date.is_some() {
            repo.save_monthly_record(record(&ids[0], date, "17", "9.46"))
                .unwrap();
        }
        let request = input(vec![
            purchase(&ids[0], "17", "9.46"),
            purchase(&ids[1], "3", "20"),
        ]);
        let before = state(&directory, &repo);
        let preview = repo
            .preview_simulation_purchases(request.clone(), &ids, today())
            .unwrap();
        assert_eq!(preview.rows[0].status, "conflict");
        assert_eq!(preview.rows[1].status, "new");
        assert!(!preview.can_confirm);
        let result = repo
            .confirm_simulation_purchases(request, &ids, today(), &preview.preview_token)
            .unwrap();
        assert_eq!(result.created_count, 0);
        assert_eq!(state(&directory, &repo), before);
    }
}

#[test]
fn different_day_quantity_or_price_does_not_silently_merge_separate_purchases() {
    let (directory, repo, ids) = setup();
    repo.save_monthly_record(record(&ids[0], Some("2026-10-19"), "17", "9.46"))
        .unwrap();
    repo.save_monthly_record(record(&ids[0], Some("2026-10-20"), "16", "9.46"))
        .unwrap();
    repo.save_monthly_record(record(&ids[0], Some("2026-10-20"), "17", "9.45"))
        .unwrap();
    let preview = repo
        .preview_simulation_purchases(input(vec![purchase(&ids[0], "17", "9.46")]), &ids, today())
        .unwrap();
    assert_eq!(preview.rows[0].status, "new");
    assert!(preview.can_confirm);
    assert_eq!(state(&directory, &repo)["receipts"], 0);
}

#[test]
fn invalid_inputs_and_unknown_fields_are_rejected_without_any_mutation() {
    let (directory, repo, ids) = setup();
    let valid = purchase(&ids[0], "1", "10");
    let before = state(&directory, &repo);
    let mut cases = vec![
        input(vec![]),
        input(vec![valid.clone(), valid.clone()]),
        input(vec![purchase("other-wallet-fund", "1", "10")]),
    ];
    for shares in [
        "0",
        "-1",
        "1.5",
        "1.0",
        "1e2",
        "+1",
        " 1 ",
        "79228162514264337593543950336",
    ] {
        cases.push(input(vec![purchase(&ids[0], shares, "10")]));
    }
    for price in [
        "0", "-1", "10.001", "1e2", "+10", ".50", "10.", "1,00", " 10 ", "1.2.3",
    ] {
        cases.push(input(vec![purchase(&ids[0], "1", price)]));
    }
    for date in [
        "2026-10-22",
        "2026-02-30",
        "2026-1-01",
        "2026-10-20T00:00:00Z",
        "0000-10-20",
        "",
    ] {
        let mut item = valid.clone();
        item.purchase_date = date.into();
        cases.push(input(vec![item]));
    }
    for source in ["", " tool", "bad\nsource"] {
        let mut request = input(vec![valid.clone()]);
        request.source_tool_id = source.into();
        cases.push(request);
    }
    for request in cases {
        assert!(
            repo.preview_simulation_purchases(request.clone(), &ids, today())
                .is_err(),
            "{request:?}"
        );
        assert!(repo
            .confirm_simulation_purchases(request, &ids, today(), "bad")
            .is_err());
        assert_eq!(state(&directory, &repo), before);
    }
    let item_json = json!({"fundId":ids[0],"purchaseDate":"2026-10-20","shares":"1","pricePerShare":"10","dividendTotal":"9"});
    assert!(serde_json::from_value::<PurchaseConfirmationItem>(item_json).is_err());
    assert!(serde_json::from_value::<PurchaseConfirmationInput>(
        json!({"sourceToolId":"x","purchases":[],"confirmedByModel":true})
    )
    .is_err());
}

#[test]
fn stale_preview_after_manual_save_is_not_allowed_to_create_or_link_silently() {
    let (directory, repo, ids) = setup();
    let request = input(vec![purchase(&ids[0], "17", "9.46")]);
    let old_preview = repo
        .preview_simulation_purchases(request.clone(), &ids, today())
        .unwrap();
    repo.save_monthly_record(record(&ids[0], Some("2026-10-20"), "17", "9.46"))
        .unwrap();
    let before = state(&directory, &repo);
    let rejected = repo
        .confirm_simulation_purchases(request.clone(), &ids, today(), &old_preview.preview_token)
        .unwrap();
    assert_eq!(rejected.rows[0].status, "conflict");
    assert_eq!(state(&directory, &repo), before);
    let refreshed = repo
        .preview_simulation_purchases(request.clone(), &ids, today())
        .unwrap();
    assert_eq!(refreshed.rows[0].status, "already_saved");
    assert_ne!(old_preview.preview_token, refreshed.preview_token);
    let result = repo
        .confirm_simulation_purchases(request, &ids, today(), &refreshed.preview_token)
        .unwrap();
    assert_eq!(result.created_count, 0);
    assert_eq!(result.matched_count, 1);
}

#[test]
fn editing_an_existing_preview_match_requires_a_new_preview_before_new_purchase() {
    let (directory, repo, ids) = setup();
    let existing = record(&ids[0], Some("2026-10-20"), "17", "9.46");
    repo.save_monthly_record(existing.clone()).unwrap();
    let request = input(vec![purchase(&ids[0], "17", "9.46")]);
    let old_preview = repo
        .preview_simulation_purchases(request.clone(), &ids, today())
        .unwrap();
    let mut edited = existing;
    edited.id = old_preview.rows[0].record_id.clone();
    edited.price_per_cota = "9.47".into();
    repo.save_monthly_record(edited).unwrap();
    let before = state(&directory, &repo);
    let rejected = repo
        .confirm_simulation_purchases(request, &ids, today(), &old_preview.preview_token)
        .unwrap();
    assert_eq!(rejected.rows[0].status, "conflict");
    assert_eq!(state(&directory, &repo), before);
}

#[test]
fn partial_selection_can_be_confirmed_later_and_changed_receipts_never_overwrite_history() {
    let (directory, repo, ids) = setup();
    let first = input(vec![purchase(&ids[0], "17", "9.46")]);
    let preview = repo
        .preview_simulation_purchases(first.clone(), &ids, today())
        .unwrap();
    repo.confirm_simulation_purchases(first, &ids, today(), &preview.preview_token)
        .unwrap();
    let second = input(vec![
        purchase(&ids[0], "17", "9.46"),
        purchase(&ids[1], "6", "20"),
    ]);
    let preview = repo
        .preview_simulation_purchases(second.clone(), &ids, today())
        .unwrap();
    assert_eq!(preview.rows[0].status, "already_confirmed");
    assert_eq!(preview.rows[1].status, "new");
    let result = repo
        .confirm_simulation_purchases(second, &ids, today(), &preview.preview_token)
        .unwrap();
    assert_eq!(result.created_count, 1);
    assert_eq!(result.matched_count, 1);
    let before = state(&directory, &repo);
    let changed = input(vec![purchase(&ids[0], "18", "9.46")]);
    let rejected = repo
        .confirm_simulation_purchases(changed, &ids, today(), "any")
        .unwrap();
    assert_eq!(rejected.rows[0].status, "conflict");
    assert_eq!(state(&directory, &repo), before);
    assert_eq!(
        repo.simulation_purchase_confirmations("real-saved-tool-id", &ids)
            .unwrap()
            .rows
            .len(),
        2
    );
}

#[test]
fn edited_confirmed_record_is_reported_as_conflict_and_never_recreated() {
    let (directory, repo, ids) = setup();
    let request = input(vec![purchase(&ids[0], "17", "9.46")]);
    let preview = repo
        .preview_simulation_purchases(request.clone(), &ids, today())
        .unwrap();
    let confirmed = repo
        .confirm_simulation_purchases(request.clone(), &ids, today(), &preview.preview_token)
        .unwrap();
    let mut edited = record(&ids[0], Some("2026-10-20"), "18", "9.46");
    edited.id = confirmed.rows[0].record_id.clone();
    repo.save_monthly_record(edited).unwrap();
    let before = state(&directory, &repo);
    let rejected = repo
        .confirm_simulation_purchases(request, &ids, today(), "old")
        .unwrap();
    assert_eq!(rejected.rows[0].status, "conflict");
    assert_eq!(
        repo.simulation_purchase_confirmations("real-saved-tool-id", &ids)
            .unwrap()
            .rows[0]
            .status,
        "conflict"
    );
    assert_eq!(state(&directory, &repo), before);
}

#[test]
fn overflow_rejects_entire_batch_and_rolls_back_receipts_and_outbox() {
    let (directory, repo, ids) = setup();
    let request = input(vec![purchase(
        &ids[0],
        "79228162514264337593543950335",
        "2",
    )]);
    let before = state(&directory, &repo);
    assert!(repo
        .preview_simulation_purchases(request, &ids, today())
        .is_err());
    // A second purchase overflows the existing wallet's shares, only after an
    // otherwise valid first new row has been inserted inside the transaction.
    let mut existing = record(
        &ids[1],
        Some("2026-10-19"),
        "79228162514264337593543950334",
        "0",
    );
    existing.dividend_per_cota = "0".into();
    repo.save_monthly_record(existing).unwrap();
    let request = input(vec![
        purchase(&ids[0], "1", "1"),
        purchase(&ids[1], "1", "1"),
    ]);
    let preview = repo
        .preview_simulation_purchases(request.clone(), &ids, today())
        .unwrap();
    let before_after_existing = state(&directory, &repo);
    assert!(repo
        .confirm_simulation_purchases(request, &ids, today(), &preview.preview_token)
        .is_err());
    assert_eq!(state(&directory, &repo), before_after_existing);
    assert_eq!(before["receipts"], 0);
}

#[test]
fn simultaneous_confirmations_have_one_purchase_and_one_receipt() {
    let (directory, repo, ids) = setup();
    let request = input(vec![purchase(&ids[0], "17", "9.46")]);
    let preview = repo
        .preview_simulation_purchases(request.clone(), &ids, today())
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let path = directory.path().join("portfolio.sqlite");
        let barrier = barrier.clone();
        let request = request.clone();
        let ids = ids.clone();
        let token = preview.preview_token.clone();
        workers.push(std::thread::spawn(move || {
            let repo = Repository::open_existing(path).unwrap();
            barrier.wait();
            repo.confirm_simulation_purchases(request, &ids, today(), &token)
                .unwrap()
        }));
    }
    let created: usize = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().created_count)
        .sum();
    assert_eq!(created, 1);
    assert_eq!(repo.snapshot().unwrap().record_count, 1);
    assert_eq!(repo.pending_changes().unwrap().len(), 3);
    assert_eq!(state(&directory, &repo)["receipts"], 1);
}

#[test]
fn schema_nine_upgrade_preserves_every_financial_row_and_adds_only_receipt_storage() {
    let (directory, repo, ids) = setup();
    repo.save_monthly_record(record(&ids[0], None, "3", "10"))
        .unwrap();
    let before = state(&directory, &repo);
    drop(repo);
    let path = directory.path().join("portfolio.sqlite");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("DROP TABLE simulation_purchase_receipts; PRAGMA user_version=9;")
        .unwrap();
    drop(db);
    let repo = Repository::open_existing(&path).unwrap();
    assert_eq!(state(&directory, &repo), before);
    let db = Connection::open(path).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        10
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM simulation_purchase_receipts WHERE source_tool_id=?1",
            params!["unused"],
            |row| row.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn database_failure_mid_batch_rolls_back_first_purchase_receipt_and_outbox() {
    let (directory, repo, ids) = setup();
    let db = Connection::open(directory.path().join("portfolio.sqlite")).unwrap();
    // IDs are generated UUIDs from this synthetic fixture, never user SQL.
    db.execute_batch(&format!(
        "CREATE TRIGGER reject_second_purchase BEFORE INSERT ON monthly_records WHEN NEW.fund_id='{}' BEGIN SELECT RAISE(ABORT, 'synthetic database failure'); END;", ids[1]
    )).unwrap();
    drop(db);
    let request = input(vec![
        purchase(&ids[0], "17", "9.46"),
        purchase(&ids[1], "6", "20"),
    ]);
    let preview = repo
        .preview_simulation_purchases(request.clone(), &ids, today())
        .unwrap();
    let before = state(&directory, &repo);
    assert!(repo
        .confirm_simulation_purchases(request, &ids, today(), &preview.preview_token)
        .is_err());
    assert_eq!(state(&directory, &repo), before);
}

#[test]
fn unrepresentable_costs_and_batch_totals_are_rejected_instead_of_rounding() {
    let (directory, repo, ids) = setup();
    let before = state(&directory, &repo);
    for request in [
        input(vec![purchase(
            &ids[0],
            "79228162514264337593543950335",
            "0.99",
        )]),
        input(vec![
            purchase(&ids[0], "1", "79228162514264337593543950335"),
            purchase(&ids[1], "1", "0.01"),
        ]),
    ] {
        assert!(repo
            .preview_simulation_purchases(request.clone(), &ids, today())
            .is_err());
        assert!(repo
            .confirm_simulation_purchases(request, &ids, today(), "any")
            .is_err());
        assert_eq!(state(&directory, &repo), before);
    }
}
