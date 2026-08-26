//! Les options pres de la monnaie cotent-elles ? Lecture seule.
use std::time::{Duration, Instant};
use ibapi::Client;
use ibapi::contracts::{Contract, SecurityType};
use ibapi::market_data::MarketDataType;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::connect("127.0.0.1:7496", 55)?;
    client.switch_market_data_type(MarketDataType::Delayed)?;
    std::thread::sleep(Duration::from_millis(1500));

    let gabarit = Contract { symbol: "NQ".into(), security_type: SecurityType::Future,
                             exchange: "CME".into(), currency: "USD".into(), ..Default::default() };
    let mut d = client.contract_details(&gabarit)?;
    d.sort_by(|a, b| a.contract.last_trade_date_or_contract_month
                      .cmp(&b.contract.last_trade_date_or_contract_month));
    let futur = d.remove(0).contract;

    let gab = Contract { symbol: futur.symbol.clone(), security_type: SecurityType::FuturesOption,
        last_trade_date_or_contract_month: "20260827".into(),
        exchange: "CME".into(), currency: "USD".into(), ..Default::default() };
    let mut details = client.contract_details(&gab)?;

    // Le spot, depuis les barres — pas le milieu de la grille.
    let spot = 29_240.0_f64;
    details.sort_by(|a, b| (a.contract.strike - spot).abs()
                    .partial_cmp(&(b.contract.strike - spot).abs()).unwrap());
    println!("spot suppose {spot} — les quatre strikes les plus proches :");

    for cible in details.iter().take(4) {
        let sub = client.market_data(&cible.contract)
            .generic_ticks(&["101", "588"]).streaming().subscribe()?;
        let fin = Instant::now() + Duration::from_secs(8);
        let (mut n, mut resume) = (0, String::new());
        while Instant::now() < fin {
            if let Some(Ok(ibapi::subscriptions::SubscriptionItem::Data(t))) =
                sub.next_timeout(Duration::from_millis(300))
            {
                n += 1;
                if let ibapi::market_data::realtime::TickTypes::OptionComputation(c) = &t
                    && c.implied_volatility.is_some() {
                    resume = format!("IV {:?} gamma {:?} sous-jacent {:?}",
                                     c.implied_volatility, c.gamma, c.underlying_price);
                }
            }
        }
        sub.cancel();
        println!("  strike {:>8} {:?} : {n:>3} messages  {resume}",
                 cible.contract.strike, cible.contract.right);
    }
    Ok(())
}
