//! Sonde manuelle : confronte le client Rust a un TWS reel.
//!
//! Exige une passerelle en marche, donc ne peut pas etre un test. Sert a
//! verifier que le cablage rend les memes chiffres qu'ib_async sur la meme
//! machine, ce qui est la seule facon honnete de valider cette couche.
//!
//! Lecture seule : aucun ordre n'est passe.

use chrono::NaiveDateTime;
use gex_core::temps::InstantReleve;
use gex_ib::client::{ADRESSE_DEFAUT, ATTENTE_LOT, CLIENT_ID_DEFAUT, Passerelle};
use gex_ib::decisions::{lots, perimetre};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let instant: NaiveDateTime = std::env::args()
        .nth(1)
        .and_then(|s| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok())
        .ok_or("usage: sonde \"AAAA-MM-JJ HH:MM:SS\" (instant UTC du releve)")?;
    let releve = InstantReleve(instant);

    println!("-- connexion (differe) --");
    let ib = Passerelle::connecter(ADRESSE_DEFAUT, CLIENT_ID_DEFAUT + 1, true)?;

    let futur = ib.front_month("NQ", "CME")?;
    println!(
        "future    : {} | conId {} | expire {}",
        futur.local_symbol, futur.contract_id, futur.last_trade_date_or_contract_month
    );

    let contrats = ib.enumerer(&futur, "CME", releve, Some(3), 0, true)?;
    println!("enumeres  : {} contrats", contrats.len());

    // Un prix d'amorce grossier suffit pour cadrer la sonde : le vrai prix
    // arrivera dans undPrice, servi par chaque tick d'option.
    let cles: Vec<_> = contrats.iter().map(|(c, _)| *c).collect();
    let amorce = cles.iter().map(|c| c.cle.strike).sum::<f64>() / cles.len().max(1) as f64;
    let retenus = perimetre(&cles, amorce, 0.005);
    // On rappelle le contrat IB complet a cote de chaque cle retenue.
    let vise: Vec<_> = retenus
        .iter()
        .filter_map(|r| contrats.iter().find(|(c, _)| c.con_id == r.con_id).cloned())
        .collect();
    println!("perimetre : {} contrats autour de {amorce:.0}", vise.len());

    let paquets = lots(&vise, 90)?;
    let mut valeurs = std::collections::HashMap::new();
    for (i, paquet) in paquets.iter().enumerate() {
        println!("  lot {}/{} ...", i + 1, paquets.len());
        valeurs.extend(ib.collecter_lot(paquet.as_slice(), ATTENTE_LOT)?);
    }

    let avec_oi = valeurs.values().filter(|v| v.open_interest.is_some()).count();
    let avec_iv = valeurs.values().filter(|v| v.iv.is_some()).count();
    let avec_gamma = valeurs.values().filter(|v| v.gamma.is_some()).count();
    let sous_jacent = valeurs.values().find_map(|v| v.sous_jacent);

    println!("\n-- ce que TWS a servi --");
    println!("  contrats repondants : {}/{}", valeurs.len(), vise.len());
    println!("  avec open interest  : {avec_oi}");
    println!("  avec IV             : {avec_iv}");
    println!("  avec gamma publie   : {avec_gamma}");
    println!("  prix du sous-jacent : {sous_jacent:?}");
    Ok(())
}
