use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletWealth {
    pub wallet: Wallet,
    pub fii_invested: String,
    pub cash_balance: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WealthTotals {
    pub fii_invested: String,
    pub cash_balance: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WealthOverview {
    pub wallets: Vec<WalletWealth>,
    pub totals: WealthTotals,
}

impl WalletCatalog {
    /// Historical FII acquisition cost and manually maintained pockets remain
    /// separate. Broker position observations may already include these same
    /// amounts and must not be added again as a new wealth component.
    pub fn wealth_overview(&self) -> Result<WealthOverview> {
        let mut wallets = Vec::new();
        let mut fii_total = Decimal::ZERO;
        let mut cash_total = Decimal::ZERO;
        for wallet in self.list()? {
            let snapshot = self.repository(Some(&wallet.id))?.snapshot()?;
            fii_total = add(fii_total, parse_decimal(&snapshot.totals.invested)?)?;
            cash_total = add(cash_total, parse_decimal(&snapshot.cash_balance)?)?;
            wallets.push(WalletWealth {
                wallet,
                fii_invested: snapshot.totals.invested,
                cash_balance: snapshot.cash_balance,
            });
        }
        Ok(WealthOverview {
            wallets,
            totals: WealthTotals {
                fii_invested: decimal_string(fii_total),
                cash_balance: decimal_string(cash_total),
            },
        })
    }
}
