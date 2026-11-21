//! Tabela única de preço dos modelos (`src/lib/price.ts`).

/// Entrada, saída e leitura de cache, em US$/MTok.
pub type PriceRow = [f64; 3];

/// Tokens: entrada, saída, escrita de cache, leitura de cache.
pub type Tok = [f64; 4];

/// `PRICE` (`price.ts:5-13`), na ordem de declaração.
pub const PRICE: [(&str, PriceRow); 7] = [
    ("opus-5-5", [4.0, 20.0, 0.2]),
    ("sonnet-5-5", [2.0, 10.0, 0.2]),
    ("haiku-5-5", [0.1, 0.5, 0.01]),
    ("fable-5-1", [10.0, 50.0, 0.25]),
    ("opus-5", [5.0, 25.0, 0.5]),
    ("opus-4-8", [5.0, 25.0, 0.5]),
    ("sonnet-5", [2.0, 10.0, 0.2]),
];

const WRITE_5M_FACTOR: f64 = 1.25;

/// Preço da chave da tabela.
pub fn price_of(key: &str) -> Option<PriceRow> {
    PRICE.iter().find(|(k, _)| *k == key).map(|(_, p)| *p)
}

fn family_of_key(key: &str) -> &str {
    key.split('-').next().unwrap_or(key)
}

/// Chaves da mais longa à mais curta (`MODEL_KEYS`, `price.ts:15`); a ordenação é estável.
fn model_keys() -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = PRICE.iter().map(|(k, _)| *k).collect();
    keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
    keys
}

/// Famílias do mais barato ao mais caro, pelo maior preço de entrada de cada uma (`MODEL_COST_ORDER`, `price.ts:19`).
pub fn model_cost_order() -> Vec<&'static str> {
    let mut families: Vec<&'static str> = Vec::new();
    for (key, _) in PRICE {
        let family = family_of_key(key);
        if !families.contains(&family) {
            families.push(family);
        }
    }
    let top = |family: &str| {
        PRICE
            .iter()
            .filter(|(k, _)| family_of_key(k) == family)
            .map(|(_, p)| p[0])
            .fold(f64::NEG_INFINITY, f64::max)
    };
    families.sort_by(|a, b| top(a).total_cmp(&top(b)));
    families
}

/// Chave da tabela para um nome de modelo (`claude-opus-5-5[1m]` → `opus-5-5`); `None` quando não é conhecido
/// (`modelKey`, `price.ts:26`).
pub fn model_key(model: Option<&str>) -> Option<&'static str> {
    let name = model.unwrap_or("").replace("claude-", "");
    model_keys().into_iter().find(|key| name.starts_with(key))
}

/// Como [`model_key`]; o apelido de família (`sonnet`) vale como a versão mais nova dela na tabela
/// (`resolveModelKey`, `price.ts:32`).
pub fn resolve_model_key(model: Option<&str>) -> Option<&'static str> {
    if let Some(known) = model_key(model) {
        return Some(known);
    }
    let family = model
        .unwrap_or("")
        .replace("claude-", "")
        .trim()
        .to_lowercase();
    PRICE
        .iter()
        .map(|(k, _)| *k)
        .find(|key| family_of_key(key) == family)
}

/// US$ de um conjunto de tokens no modelo da chave; a escrita de cache entra como a de 5 min (`tokCost`,
/// `price.ts:42`).
pub fn tok_cost(key: &str, tok: Tok) -> Option<f64> {
    let [input, output, write, read] = tok;
    let [in_price, out_price, read_price] = price_of(key)?;
    Some(
        (input * in_price
            + output * out_price
            + read * read_price
            + write * in_price * WRITE_5M_FACTOR)
            / 1e6,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chave_do_modelo_ignora_prefixo_e_sufixo() {
        assert_eq!(model_key(Some("claude-opus-5-5[1m]")), Some("opus-5-5"));
        assert_eq!(model_key(Some("claude-opus-5-1")), Some("opus-5"));
        assert_eq!(model_key(Some("outro")), None);
        assert_eq!(model_key(None), None);
    }

    #[test]
    fn apelido_de_familia_vale_a_versao_mais_nova_da_tabela() {
        assert_eq!(resolve_model_key(Some("sonnet")), Some("sonnet-5-5"));
        assert_eq!(resolve_model_key(Some("Opus ")), Some("opus-5-5"));
        assert_eq!(resolve_model_key(Some("claude-desconhecido")), None);
    }

    #[test]
    fn ordem_de_custo_das_familias() {
        assert_eq!(model_cost_order(), ["haiku", "sonnet", "opus", "fable"]);
    }

    #[test]
    fn custo_de_tokens_soma_escrita_como_5_minutos() {
        let million = 1_000_000.0;
        assert_eq!(tok_cost("sonnet-5-5", [million, 0.0, 0.0, 0.0]), Some(2.0));
        assert_eq!(tok_cost("opus-5-5", [0.0, 0.0, million, 0.0]), Some(5.0));
        assert_eq!(tok_cost("nada", [1.0, 1.0, 1.0, 1.0]), None);
    }
}
