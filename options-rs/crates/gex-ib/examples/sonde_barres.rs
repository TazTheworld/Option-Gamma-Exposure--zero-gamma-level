//! Sonde manuelle : les barres du future, contre un TWS reel.
//! Lecture seule.
use gex_ib::barres::PROFONDEUR_JOURS;
use gex_ib::client::{ADRESSE_DEFAUT, CLIENT_ID_DEFAUT, Passerelle};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ib = Passerelle::connecter(ADRESSE_DEFAUT, CLIENT_ID_DEFAUT + 3, true)?;
    let futur = ib.front_month("NQ", "CME")?;
    println!("future : {} | multiplicateur x{}", futur.local_symbol, futur.multiplier);

    let barres = ib.barres(&futur, PROFONDEUR_JOURS)?;
    println!("\n{} barres d'une minute", barres.len());
    if barres.is_empty() {
        println!("AUCUNE BARRE — l'appel a repondu mais n'a rien servi.");
        return Ok(());
    }

    for b in barres.iter().take(3) {
        println!("  {}  O {:.2}  H {:.2}  L {:.2}  C {:.2}  V {:.0}",
                 b.instant.format("%Y-%m-%d %H:%M"), b.open, b.high, b.low, b.close, b.volume);
    }
    println!("  ...");
    for b in barres.iter().rev().take(3).collect::<Vec<_>>().into_iter().rev() {
        println!("  {}  O {:.2}  H {:.2}  L {:.2}  C {:.2}  V {:.0}",
                 b.instant.format("%Y-%m-%d %H:%M"), b.open, b.high, b.low, b.close, b.volume);
    }

    let premiere = barres.first().unwrap().instant;
    let derniere = barres.last().unwrap().instant;
    println!("\ncouverture : {} -> {}", premiere, derniere);
    println!("triees     : {}", barres.windows(2).all(|p| p[0].instant < p[1].instant));
    println!("doublons   : {}", barres.len() - barres.iter().map(|b| b.instant)
                                       .collect::<std::collections::BTreeSet<_>>().len());
    // Le piege maison de ce depot : une heure qui se derobe.
    println!("\nl'instant est-il en UTC ? comparer a l'heure UTC courante :");
    println!("  derniere barre : {}", derniere.format("%Y-%m-%d %H:%M"));
    Ok(())
}
