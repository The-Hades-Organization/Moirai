//! View-model do app: formatação, ordenação, filtro e tons, sem janela nem I/O.

use moirai_core::budget::BudgetLevel;
use moirai_core::limits::{Atual, Faixa, Janela, LimiteDoPlano};
use moirai_core::model::{Band, HistoryUnit, Mode, UnitEstimate};

use hades_ui::Tone;

/// Telas da navegação lateral (`design/moirai.html`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tela {
    Estimar,
    Historico,
    Teto,
    Janelas,
}

impl Tela {
    pub const TODAS: [Tela; 4] = [Tela::Estimar, Tela::Historico, Tela::Teto, Tela::Janelas];

    pub fn rotulo(self) -> &'static str {
        match self {
            Tela::Estimar => "Estimar plano",
            Tela::Historico => "Histórico",
            Tela::Teto => "Teto ao vivo",
            Tela::Janelas => "Janelas",
        }
    }

    pub fn de_rotulo(rotulo: &str) -> Option<Tela> {
        Tela::TODAS.into_iter().find(|t| t.rotulo() == rotulo)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direcao {
    Asc,
    Desc,
}

/// Ordenação por coluna: o primeiro clique ordena de forma crescente, o segundo inverte, outra coluna recomeça.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ordem {
    pub coluna: usize,
    pub direcao: Direcao,
}

impl Ordem {
    pub fn alternar(atual: Option<Ordem>, coluna: usize) -> Ordem {
        match atual {
            Some(o) if o.coluna == coluna => Ordem {
                coluna,
                direcao: match o.direcao {
                    Direcao::Asc => Direcao::Desc,
                    Direcao::Desc => Direcao::Asc,
                },
            },
            _ => Ordem {
                coluna,
                direcao: Direcao::Asc,
            },
        }
    }
}

/// Posição do seletor projeto / skill.
pub fn modo_de_indice(indice: usize) -> Mode {
    if indice == 1 {
        Mode::Skill
    } else {
        Mode::Projeto
    }
}

pub fn indice_do_modo(mode: Mode) -> usize {
    match mode {
        Mode::Projeto => 0,
        Mode::Skill => 1,
    }
}

fn agrupar_milhar(inteiro: &str) -> String {
    let mut out = String::new();
    for (i, c) in inteiro.chars().enumerate() {
        if i > 0 && (inteiro.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// `1234,5` → `1.234,50`; vírgula decimal, ponto de milhar.
pub fn numero(valor: f64, casas: usize) -> String {
    let texto = format!("{valor:.casas$}");
    let (sinal, resto) = texto
        .strip_prefix('-')
        .map_or(("", texto.as_str()), |r| ("-", r));
    let (inteiro, frac) = resto.split_once('.').unwrap_or((resto, ""));
    let base = format!("{sinal}{}", agrupar_milhar(inteiro));
    if frac.is_empty() {
        base
    } else {
        format!("{base},{frac}")
    }
}

pub fn usd(valor: f64) -> String {
    format!("US$ {}", numero(valor, 2))
}

/// Custo sem prefixo, para células de tabela: `6,10`.
pub fn usd_celula(valor: Option<f64>) -> String {
    valor.map_or_else(|| "—".to_owned(), |v| numero(v, 2))
}

/// Minutos como `9h40` ou `45min`.
pub fn duracao(minutos: f64) -> String {
    let total = minutos.round().max(0.0) as i64;
    if total < 60 {
        format!("{total}min")
    } else {
        format!("{}h{:02}", total / 60, total % 60)
    }
}

pub fn duracao_celula(minutos: Option<f64>) -> String {
    minutos.map_or_else(|| "—".to_owned(), duracao)
}

/// Segundos de relógio como minutos arredondados.
pub fn duracao_s(segundos: Option<f64>) -> String {
    duracao_celula(segundos.map(|s| s / 60.0))
}

pub fn faixa_letra(band: Option<Band>) -> &'static str {
    match band {
        Some(Band::Pequena) => "P",
        Some(Band::Media) => "M",
        Some(Band::Grande) => "G",
        None => "—",
    }
}

fn ordem_da_faixa(band: Option<Band>) -> u8 {
    match band {
        Some(Band::Pequena) => 0,
        Some(Band::Media) => 1,
        Some(Band::Grande) => 2,
        None => 3,
    }
}

/// KPIs do plano: soma das medianas e dos p75 (o teto de custo é o p75), duração e tamanho da base.
#[derive(Clone, Debug, PartialEq)]
pub struct Kpis {
    pub custo_p50: Option<f64>,
    pub custo_p75: Option<f64>,
    pub duracao_p50_min: Option<f64>,
    pub base: usize,
}

fn soma(valores: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    valores.fold(None, |acc, v| match (acc, v) {
        (a, Some(x)) => Some(a.unwrap_or(0.0) + x),
        (a, None) => a,
    })
}

pub fn kpis(estimativas: &[UnitEstimate], base: usize) -> Kpis {
    Kpis {
        custo_p50: soma(estimativas.iter().map(|e| e.cost_usd.map(|s| s.median))),
        custo_p75: soma(estimativas.iter().map(|e| e.cost_usd.map(|s| s.p75))),
        duracao_p50_min: soma(estimativas.iter().map(|e| e.wall_min.map(|s| s.median))),
        base,
    }
}

pub const COLUNAS_ESTIMAR: [&str; 7] = [
    "Unidade",
    "Papel",
    "Faixa",
    "US$ p50",
    "US$ p75",
    "Duração",
    "Gate",
];

/// Linha da tabela "Estimar plano".
#[derive(Clone, Debug, PartialEq)]
pub struct LinhaEstimar {
    pub unidade: String,
    pub papel: String,
    pub faixa: Option<Band>,
    pub p50: Option<f64>,
    pub p75: Option<f64>,
    pub duracao_min: Option<f64>,
    pub gate_min: Option<f64>,
}

pub fn linhas_estimar(estimativas: &[UnitEstimate]) -> Vec<LinhaEstimar> {
    estimativas
        .iter()
        .map(|e| LinhaEstimar {
            unidade: e.unit.clone(),
            papel: e.role.clone(),
            faixa: e.band,
            p50: e.cost_usd.map(|s| s.median),
            p75: e.cost_usd.map(|s| s.p75),
            duracao_min: e.wall_min.map(|s| s.median),
            gate_min: e.gate_min.map(|s| s.median),
        })
        .collect()
}

fn cmp_opcional(a: Option<f64>, b: Option<f64>) -> std::cmp::Ordering {
    use std::cmp::Ordering::{Equal, Greater, Less};
    match (a, b) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Equal),
        (Some(_), None) => Less,
        (None, Some(_)) => Greater,
        (None, None) => Equal,
    }
}

/// Valores ausentes ficam sempre no fim, em qualquer direção.
fn aplicar_direcao(
    ordem: std::cmp::Ordering,
    direcao: Direcao,
    ausente: bool,
) -> std::cmp::Ordering {
    if ausente {
        return ordem;
    }
    match direcao {
        Direcao::Asc => ordem,
        Direcao::Desc => ordem.reverse(),
    }
}

pub fn ordenar_estimar(linhas: &mut [LinhaEstimar], ordem: Option<Ordem>) {
    let Some(o) = ordem else { return };
    linhas.sort_by(|a, b| {
        let (cmp, ausente) = match o.coluna {
            0 => (a.unidade.cmp(&b.unidade), false),
            1 => (a.papel.cmp(&b.papel), false),
            2 => (
                ordem_da_faixa(a.faixa).cmp(&ordem_da_faixa(b.faixa)),
                a.faixa.is_none() || b.faixa.is_none(),
            ),
            c => {
                let (x, y) = match c {
                    3 => (a.p50, b.p50),
                    4 => (a.p75, b.p75),
                    5 => (a.duracao_min, b.duracao_min),
                    _ => (a.gate_min, b.gate_min),
                };
                (cmp_opcional(x, y), x.is_none() || y.is_none())
            }
        };
        aplicar_direcao(cmp, o.direcao, ausente)
    });
}

pub fn celulas_estimar(l: &LinhaEstimar) -> Vec<String> {
    vec![
        l.unidade.clone(),
        l.papel.clone(),
        faixa_letra(l.faixa).to_owned(),
        usd_celula(l.p50),
        usd_celula(l.p75),
        duracao_celula(l.duracao_min),
        duracao_celula(l.gate_min),
    ]
}

pub const COLUNAS_HISTORICO: [&str; 8] = [
    "Unidade",
    "Papel",
    "Modo",
    "Faixa",
    "US$ real",
    "Duração",
    "Gate",
    "Encerrada",
];

/// Filtros do Histórico: conjunto vazio de uma dimensão significa "sem filtro" nela.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filtros {
    pub papeis: Vec<String>,
    pub modos: Vec<Mode>,
    pub faixas: Vec<Band>,
}

pub fn alternar<T: PartialEq>(conjunto: &mut Vec<T>, valor: T) {
    if let Some(i) = conjunto.iter().position(|v| *v == valor) {
        conjunto.remove(i);
    } else {
        conjunto.push(valor);
    }
}

impl Filtros {
    pub fn vazio(&self) -> bool {
        self.papeis.is_empty() && self.modos.is_empty() && self.faixas.is_empty()
    }

    pub fn aceita(&self, u: &HistoryUnit) -> bool {
        (self.papeis.is_empty() || self.papeis.contains(&u.role))
            && (self.modos.is_empty() || self.modos.contains(&u.mode))
            && (self.faixas.is_empty() || u.band.is_some_and(|b| self.faixas.contains(&b)))
    }
}

/// Os chips da barra, na ordem fixa: papéis encontrados (ordenados), modos, faixas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chip {
    Papel(String),
    Modo(Mode),
    Faixa(Band),
}

pub fn chips_do_historico(unidades: &[HistoryUnit]) -> Vec<Chip> {
    let mut papeis: Vec<String> = unidades.iter().map(|u| u.role.clone()).collect();
    papeis.sort();
    papeis.dedup();
    let mut chips: Vec<Chip> = papeis.into_iter().map(Chip::Papel).collect();
    chips.extend([Chip::Modo(Mode::Projeto), Chip::Modo(Mode::Skill)]);
    chips.extend([
        Chip::Faixa(Band::Pequena),
        Chip::Faixa(Band::Media),
        Chip::Faixa(Band::Grande),
    ]);
    chips
}

pub fn rotulo_do_chip(chip: &Chip) -> String {
    match chip {
        Chip::Papel(p) => p.clone(),
        Chip::Modo(m) => m.as_str().to_owned(),
        Chip::Faixa(b) => format!("faixa {}", faixa_letra(Some(*b))),
    }
}

pub fn chip_ativo(filtros: &Filtros, chip: &Chip) -> bool {
    match chip {
        Chip::Papel(p) => filtros.papeis.contains(p),
        Chip::Modo(m) => filtros.modos.contains(m),
        Chip::Faixa(b) => filtros.faixas.contains(b),
    }
}

pub fn alternar_chip(filtros: &mut Filtros, chip: &Chip) {
    match chip {
        Chip::Papel(p) => alternar(&mut filtros.papeis, p.clone()),
        Chip::Modo(m) => alternar(&mut filtros.modos, *m),
        Chip::Faixa(b) => alternar(&mut filtros.faixas, *b),
    }
}

/// Unidades que passam no filtro, ordenadas; devolve também a posição de cada uma em `unidades`.
pub fn visiveis(unidades: &[HistoryUnit], filtros: &Filtros, ordem: Option<Ordem>) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..unidades.len())
        .filter(|&i| filtros.aceita(&unidades[i]))
        .collect();
    let Some(o) = ordem else { return idx };
    idx.sort_by(|&i, &j| {
        let (a, b) = (&unidades[i], &unidades[j]);
        let (cmp, ausente) = match o.coluna {
            0 => (a.name.cmp(&b.name), false),
            1 => (a.role.cmp(&b.role), false),
            2 => (a.mode.as_str().cmp(b.mode.as_str()), false),
            3 => (
                ordem_da_faixa(a.band).cmp(&ordem_da_faixa(b.band)),
                a.band.is_none() || b.band.is_none(),
            ),
            4 => (
                cmp_opcional(a.cost_usd, b.cost_usd),
                a.cost_usd.is_none() || b.cost_usd.is_none(),
            ),
            5 => (
                cmp_opcional(a.wall_s, b.wall_s),
                a.wall_s.is_none() || b.wall_s.is_none(),
            ),
            6 => (
                a.gate_s
                    .partial_cmp(&b.gate_s)
                    .unwrap_or(std::cmp::Ordering::Equal),
                false,
            ),
            _ => (a.done_at.cmp(&b.done_at), false),
        };
        aplicar_direcao(cmp, o.direcao, ausente)
    });
    idx
}

pub fn celulas_historico(u: &HistoryUnit) -> Vec<String> {
    vec![
        u.name.clone(),
        u.role.clone(),
        u.mode.as_str().to_owned(),
        faixa_letra(u.band).to_owned(),
        usd_celula(u.cost_usd),
        duracao_s(u.wall_s),
        duracao_s(Some(u.gate_s)),
        u.done_at.get(..10).unwrap_or(&u.done_at).to_owned(),
    ]
}

/// Um medidor de janela: fração usada, texto e tom.
#[derive(Clone, Debug, PartialEq)]
pub struct MedidorJanela {
    pub janela: Janela,
    pub fracao: f32,
    pub texto: String,
    pub tom: Tone,
}

/// `bad` só com o limite estourado de fato (100% medido); `warn` quando a projeção do plano estoura a janela
/// (decisão do design).
pub fn tom_da_janela(atual: Option<&Atual>, limite: &LimiteDoPlano) -> Tone {
    if atual.is_some_and(|a| a.pct >= 100.0) {
        Tone::Bad
    } else if limite.cabe == Some(false) {
        Tone::Warn
    } else {
        Tone::Neutral
    }
}

fn pct_texto(p: f64) -> String {
    format!("{}%", numero(p, 0))
}

pub fn medidor(janela: Janela, limite: &LimiteDoPlano) -> MedidorJanela {
    let atual = limite.atual.as_ref();
    let tom = tom_da_janela(atual, limite);
    let (fracao, base) = match atual {
        Some(a) => ((a.pct / 100.0).clamp(0.0, 1.0) as f32, pct_texto(a.pct)),
        None => (0.0, "sem medida".to_owned()),
    };
    let projecao = limite
        .plano
        .filter(|_| tom == Tone::Warn)
        .zip(atual)
        .map(|(f, a): (Faixa, &Atual)| format!(" · o plano leva a ~{}", pct_texto(a.pct + f.p75)));
    MedidorJanela {
        janela,
        fracao,
        texto: format!("{base}{}", projecao.unwrap_or_default()),
        tom,
    }
}

pub const COLUNAS_TETO: [&str; 7] = [
    "Unidade", "Plano", "Estado", "US$ real", "US$ p75", "% do p75", "Nível",
];

/// Uma unidade com custo real medido contra o p75 da estimativa (`GET /api/moirai/plan`, `src/panel/moirai.ts:104`).
#[derive(Clone, Debug, PartialEq)]
pub struct LinhaTeto {
    pub unidade: String,
    pub plano: String,
    pub aprovado: bool,
    pub estado: Option<String>,
    pub custo_real: f64,
    pub p75: Option<f64>,
    pub pct: Option<f64>,
    pub nivel: Option<BudgetLevel>,
}

fn texto_de<'a>(v: &'a serde_json::Value, chave: &str) -> Option<&'a str> {
    v.get(chave).and_then(serde_json::Value::as_str)
}

/// Linhas do teto a partir do JSON de `MoiraiRoutes::plan`: só unidade com agente (custo real), da mais estourada
/// para a menos; sem p75 vão para o fim.
pub fn linhas_teto(plano: &serde_json::Value) -> Vec<LinhaTeto> {
    let mut linhas = Vec::new();
    let planos = plano.get("plans").and_then(serde_json::Value::as_array);
    for p in planos.into_iter().flatten() {
        let registro = texto_de(p, "registry").unwrap_or_default();
        let aprovado = p.get("approved").and_then(serde_json::Value::as_bool) == Some(true);
        let unidades = p.get("units").and_then(serde_json::Value::as_array);
        for u in unidades.into_iter().flatten() {
            let Some(real) = u.get("real").filter(|r| r.is_object()) else {
                continue;
            };
            let nivel = match texto_de(u, "level") {
                Some("ask") => Some(BudgetLevel::Ask),
                Some("warn") => Some(BudgetLevel::Warn),
                Some("ok") => Some(BudgetLevel::Ok),
                _ => None,
            };
            linhas.push(LinhaTeto {
                unidade: texto_de(u, "unit").unwrap_or_default().to_owned(),
                plano: registro.to_owned(),
                aprovado,
                estado: texto_de(real, "state").map(str::to_owned),
                custo_real: real
                    .get("costUsd")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0),
                p75: u
                    .get("estimate")
                    .and_then(|e| e.get("costP75"))
                    .and_then(serde_json::Value::as_f64),
                pct: u.get("pct").and_then(serde_json::Value::as_f64),
                nivel,
            });
        }
    }
    linhas.sort_by(|a, b| match (a.pct, b.pct) {
        (Some(x), Some(y)) => y.total_cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => b.custo_real.total_cmp(&a.custo_real),
    });
    linhas
}

/// `warn` ao passar do limiar de aviso e `bad` ao passar do limiar de pergunta (o custo já foi medido).
pub fn tom_do_nivel(nivel: Option<BudgetLevel>) -> Tone {
    match nivel {
        Some(BudgetLevel::Ask) => Tone::Bad,
        Some(BudgetLevel::Warn) => Tone::Warn,
        Some(BudgetLevel::Ok) => Tone::Ok,
        None => Tone::Neutral,
    }
}

pub fn rotulo_do_nivel(nivel: Option<BudgetLevel>) -> &'static str {
    match nivel {
        Some(BudgetLevel::Ask) => "estourou",
        Some(BudgetLevel::Warn) => "no teto",
        Some(BudgetLevel::Ok) => "dentro",
        None => "sem estimativa",
    }
}

pub fn celulas_teto(l: &LinhaTeto) -> Vec<String> {
    vec![
        l.unidade.clone(),
        l.plano.clone(),
        l.estado.clone().unwrap_or_else(|| "—".to_owned()),
        usd(l.custo_real),
        usd_celula(l.p75),
        l.pct.map_or_else(|| "—".to_owned(), pct_texto),
        rotulo_do_nivel(l.nivel).to_owned(),
    ]
}

/// Limiares de aviso e pergunta do painel (`GET /api/moirai/plan`: `warnPct`, `askPct`).
pub fn limiares(plano: &serde_json::Value) -> Option<(f64, f64)> {
    Some((
        plano.get("warnPct")?.as_f64()?,
        plano.get("askPct")?.as_f64()?,
    ))
}

/// Quanto falta para a janela renovar (`resets_at` em segundos desde a época, `agora_ms` em ms).
pub fn renovacao(resets_at_s: f64, agora_ms: f64) -> String {
    let falta_min = (resets_at_s - agora_ms / 1000.0) / 60.0;
    if falta_min <= 0.0 {
        return "renovando agora".to_owned();
    }
    let total = falta_min.round() as i64;
    match (total / 1440, total % 1440 / 60, total % 60) {
        (0, 0, m) => format!("renova em {m}min"),
        (0, h, m) => format!("renova em {h}h{m:02}"),
        (d, h, _) => format!("renova em {d}d {h}h"),
    }
}

/// Cartão de uma janela: medidor, renovação e quanto o plano aberto consome.
#[derive(Clone, Debug, PartialEq)]
pub struct CartaoJanela {
    pub medidor: MedidorJanela,
    pub renovacao: String,
    pub plano: String,
}

pub fn cartao_da_janela(janela: Janela, limite: &LimiteDoPlano, agora_ms: f64) -> CartaoJanela {
    let renovacao = limite.atual.as_ref().map_or_else(
        || "sem medida".to_owned(),
        |a| renovacao(a.resets_at, agora_ms),
    );
    let plano = match limite.plano {
        None => "plano aberto sem estimativa calibrada".to_owned(),
        Some(f) => format!(
            "plano aberto: ~{}% a ~{}%{}",
            numero(f.p25, 0),
            numero(f.p75, 0),
            if limite.completo { "" } else { " (parcial)" }
        ),
    };
    CartaoJanela {
        medidor: medidor(janela, limite),
        renovacao,
        plano,
    }
}

/// Linha de status das janelas, no estilo do terminal: `5 h 42% · semana 18%`.
pub fn linha_das_janelas(limites: &[(Janela, Option<&Atual>)]) -> String {
    limites
        .iter()
        .map(|(j, a)| {
            format!(
                "{} {}",
                j.rotulo(),
                a.map_or_else(|| "sem medida".to_owned(), |a| pct_texto(a.pct))
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Assinatura do plano no disco (tamanho e mtime) para o watcher só reestimar quando mudou de fato.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assinatura {
    pub bytes: u64,
    pub mtime_ms: i128,
}

pub fn mudou(antes: Option<Assinatura>, agora: Option<Assinatura>) -> bool {
    antes != agora
}

/// Colunas de número: custo, duração e gate (alinhadas à direita, em mono).
pub fn coluna_numerica(nome: &str) -> bool {
    nome.starts_with("US$") || nome == "% do p75" || nome == "Duração" || nome == "Gate"
}

/// Atalho: teclas 1 a 4 trocam de tela, como o clique no `Rail`.
pub fn tela_da_tecla(tecla: &str) -> Option<Tela> {
    let n: usize = tecla.parse().ok()?;
    Tela::TODAS.get(n.checked_sub(1)?).copied()
}

#[cfg(test)]
mod tests;
