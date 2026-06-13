use crate::core::Core;
use anyhow::Result;
use cursive::{
    Cursive,
    traits::*,
    views::{Dialog, EditView, LinearLayout, Panel, ScrollView, SelectView, TextView},
};
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::time::{self, Duration};

pub fn run(core: Arc<Core>, rt: Handle) -> Result<()> {
    let mut siv = cursive::default();

    let node_addr = core.node_addr().to_string();

    let left = Panel::new(
        LinearLayout::vertical()
            .child(TextView::new("Balance"))
            .child(TextView::new("—").with_name("balance"))
            .child(TextView::new(""))
            .child(TextView::new("Node"))
            .child(TextView::new(node_addr)),
    )
    .title("Wallet")
    .fixed_width(28);

    let right = Panel::new(
        ScrollView::new(TextView::new("Connecting...").with_name("utxo_list")).full_height(),
    )
    .title("UTXOs")
    .full_width();

    let status = Panel::new(
        TextView::new("[S]end  [C]ontacts  [R]efresh  [Q]uit").with_name("status"),
    )
    .fixed_height(3);

    let layout = LinearLayout::vertical()
        .child(
            LinearLayout::horizontal()
                .child(left)
                .child(right)
                .full_height(),
        )
        .child(status);

    siv.add_fullscreen_layer(layout);

    // Background auto-refresh every 10 s
    {
        let cb = siv.cb_sink().clone();
        let c = Arc::clone(&core);
        rt.spawn(async move {
            let mut interval = time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                do_refresh(Arc::clone(&c), cb.clone()).await;
            }
        });
    }

    // [S]end
    {
        let (c, r) = (Arc::clone(&core), rt.clone());
        siv.add_global_callback('s', move |s| show_send(s, Arc::clone(&c), r.clone()));
    }

    // [C]ontacts
    {
        let c = Arc::clone(&core);
        siv.add_global_callback('c', move |s| show_contacts(s, Arc::clone(&c)));
    }

    // [R]efresh
    {
        let (c, r) = (Arc::clone(&core), rt.clone());
        siv.add_global_callback('r', move |s| {
            set_status(s, "Refreshing...");
            let cb = s.cb_sink().clone();
            r.spawn(do_refresh(Arc::clone(&c), cb));
        });
    }

    // [Q]uit
    siv.add_global_callback('q', |s| s.quit());

    // Initial fetch
    {
        let cb = siv.cb_sink().clone();
        let c = Arc::clone(&core);
        rt.spawn(do_refresh(c, cb));
    }

    siv.run();
    Ok(())
}

// ---------------------------------------------------------------------------
// Background refresh
// ---------------------------------------------------------------------------

async fn do_refresh(core: Arc<Core>, cb: cursive::CbSink) {
    let ok = core.fetch_utxos().await.is_ok();
    let balance = core.get_balance().await;
    let utxos = core.utxos.read().await.clone();

    cb.send(Box::new(move |s: &mut Cursive| {
        s.call_on_name("balance", |v: &mut TextView| {
            v.set_content(format!("{} coins", balance));
        });

        let list = if utxos.is_empty() {
            "No UTXOs found.".to_string()
        } else {
            utxos
                .iter()
                .map(|(out, pending)| {
                    let tag = if *pending { "[pending]" } else { "[unspent]" };
                    format!("{} {} coins", tag, out.value)
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        s.call_on_name("utxo_list", |v: &mut TextView| {
            v.set_content(list);
        });

        let suffix = if ok { "synced" } else { "node unreachable" };
        set_status(
            s,
            &format!("[S]end  [C]ontacts  [R]efresh  [Q]uit  — {}", suffix),
        );
    }))
    .ok();
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

fn show_contacts(siv: &mut Cursive, core: Arc<Core>) {
    if core.config.contacts.is_empty() {
        siv.add_layer(Dialog::info("No contacts configured."));
        return;
    }
    let text = core
        .config
        .contacts
        .iter()
        .map(|c| format!("  {}", c.name))
        .collect::<Vec<_>>()
        .join("\n");
    siv.add_layer(
        Dialog::around(TextView::new(text))
            .title("Contacts")
            .button("Close", |s| {
                s.pop_layer();
            }),
    );
}

fn show_send(siv: &mut Cursive, core: Arc<Core>, rt: Handle) {
    let contacts: Vec<String> = core
        .config
        .contacts
        .iter()
        .map(|c| c.name.clone())
        .collect();

    if contacts.is_empty() {
        siv.add_layer(Dialog::info("No contacts configured."));
        return;
    }

    let mut select = SelectView::<String>::new();
    for name in &contacts {
        select.add_item(name.clone(), name.clone());
    }

    let (c, r) = (Arc::clone(&core), rt.clone());
    siv.add_layer(
        Dialog::new()
            .title("Send")
            .content(
                LinearLayout::vertical()
                    .child(TextView::new("Recipient:"))
                    .child(select.with_name("send_contact").fixed_height(4).scrollable())
                    .child(TextView::new(""))
                    .child(TextView::new("Amount (coins):"))
                    .child(EditView::new().with_name("send_amount").fixed_width(24)),
            )
            .button("Send", move |s| on_send(s, Arc::clone(&c), r.clone()))
            .button("Cancel", |s| {
                s.pop_layer();
            }),
    );
}

fn on_send(siv: &mut Cursive, core: Arc<Core>, rt: Handle) {
    let contact = siv
        .call_on_name("send_contact", |v: &mut SelectView<String>| {
            v.selection().map(|rc| (*rc).clone())
        })
        .flatten();

    let amount_str = siv
        .call_on_name("send_amount", |v: &mut EditView| v.get_content().to_string())
        .unwrap_or_default();

    let amount: u64 = match amount_str.trim().parse() {
        Ok(v) => v,
        Err(_) => {
            siv.add_layer(Dialog::info("Invalid amount. Enter a whole number of coins."));
            return;
        }
    };

    let name = match contact {
        Some(n) => n,
        None => {
            siv.add_layer(Dialog::info("No recipient selected."));
            return;
        }
    };

    siv.pop_layer();
    let cb = siv.cb_sink().clone();

    rt.spawn(async move {
        let result: anyhow::Result<()> = async {
            let recipient = core
                .config
                .contacts
                .iter()
                .find(|r| r.name == name)
                .ok_or_else(|| anyhow::anyhow!("Contact not found"))?;
            let loaded = recipient.load()?;
            let tx = core.create_transaction(&loaded.public_key, amount).await?;
            core.tx_sender
                .send(tx)
                .await
                .map_err(|e| anyhow::anyhow!("Queue error: {e}"))?;
            Ok(())
        }
        .await;

        let msg = match &result {
            Ok(_) => format!("Queued: {} coins → {}", amount, name),
            Err(e) => format!("Error: {e}"),
        };

        cb.send(Box::new(move |s: &mut Cursive| {
            set_status(
                s,
                &format!("[S]end  [C]ontacts  [R]efresh  [Q]uit  — {}", msg),
            );
        }))
        .ok();
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn set_status(siv: &mut Cursive, text: &str) {
    siv.call_on_name("status", |v: &mut TextView| {
        v.set_content(text);
    });
}
