//! Defines database & queries for macro expansion.

use std::sync::Arc;

use mbe::MacroRules;
use ra_db::{salsa, SourceDatabase};
use ra_parser::FragmentKind;
use ra_prof::profile;
use ra_syntax::{AstNode, Parse, SyntaxKind::*, SyntaxNode};

use crate::{
    ast_id_map::AstIdMap, BuiltinDeriveExpander, BuiltinFnLikeExpander, EagerCallLoc, EagerMacroId,
    HirFileId, HirFileIdRepr, LazyMacroId, MacroCallId, MacroCallLoc, MacroDefId, MacroDefKind,
    MacroFile,
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TokenExpander {
    MacroRules(mbe::MacroRules),
    Builtin(BuiltinFnLikeExpander),
    BuiltinDerive(BuiltinDeriveExpander),
}

impl TokenExpander {
    pub fn expand(
        &self,
        db: &dyn AstDatabase,
        id: LazyMacroId,
        tt: &tt::Subtree,
    ) -> Result<tt::Subtree, mbe::ExpandError> {
        match self {
            TokenExpander::MacroRules(it) => it.expand(tt),
            TokenExpander::Builtin(it) => it.expand(db, id, tt),
            TokenExpander::BuiltinDerive(it) => it.expand(db, id, tt),
        }
    }

    pub fn map_id_down(&self, id: tt::TokenId) -> tt::TokenId {
        match self {
            TokenExpander::MacroRules(it) => it.map_id_down(id),
            TokenExpander::Builtin(..) => id,
            TokenExpander::BuiltinDerive(..) => id,
        }
    }

    pub fn map_id_up(&self, id: tt::TokenId) -> (tt::TokenId, mbe::Origin) {
        match self {
            TokenExpander::MacroRules(it) => it.map_id_up(id),
            TokenExpander::Builtin(..) => (id, mbe::Origin::Call),
            TokenExpander::BuiltinDerive(..) => (id, mbe::Origin::Call),
        }
    }
}

// FIXME: rename to ExpandDatabase
#[salsa::query_group(AstDatabaseStorage)]
pub trait AstDatabase: SourceDatabase {
    fn ast_id_map(&self, file_id: HirFileId) -> Arc<AstIdMap>;

    #[salsa::transparent]
    fn parse_or_expand(&self, file_id: HirFileId) -> Option<SyntaxNode>;

    #[salsa::interned]
    fn intern_macro(&self, macro_call: MacroCallLoc) -> LazyMacroId;
    fn macro_arg(&self, id: MacroCallId) -> Option<Arc<(tt::Subtree, mbe::TokenMap)>>;
    fn macro_def(&self, id: MacroDefId) -> Option<Arc<(TokenExpander, mbe::TokenMap)>>;
    fn parse_macro(&self, macro_file: MacroFile)
        -> Option<(Parse<SyntaxNode>, Arc<mbe::TokenMap>)>;
    fn macro_expand(&self, macro_call: MacroCallId) -> Result<Arc<tt::Subtree>, String>;

    #[salsa::interned]
    fn intern_eager_expansion(&self, eager: EagerCallLoc) -> EagerMacroId;
}

pub(crate) fn ast_id_map(db: &dyn AstDatabase, file_id: HirFileId) -> Arc<AstIdMap> {
    let map =
        db.parse_or_expand(file_id).map_or_else(AstIdMap::default, |it| AstIdMap::from_source(&it));
    Arc::new(map)
}

pub(crate) fn macro_def(
    db: &dyn AstDatabase,
    id: MacroDefId,
) -> Option<Arc<(TokenExpander, mbe::TokenMap)>> {
    match id.kind {
        MacroDefKind::Declarative => {
            let macro_call = id.ast_id?.to_node(db);
            let arg = macro_call.token_tree()?;
            let (tt, tmap) = mbe::ast_to_token_tree(&arg).or_else(|| {
                log::warn!("fail on macro_def to token tree: {:#?}", arg);
                None
            })?;
            let rules = match MacroRules::parse(&tt) {
                Ok(it) => it,
                Err(err) => {
                    log::warn!("fail on macro_def parse: error: {:#?} {:#?}", err, tt);
                    return None;
                }
            };
            Some(Arc::new((TokenExpander::MacroRules(rules), tmap)))
        }
        MacroDefKind::BuiltIn(expander) => {
            Some(Arc::new((TokenExpander::Builtin(expander), mbe::TokenMap::default())))
        }
        MacroDefKind::BuiltInDerive(expander) => {
            Some(Arc::new((TokenExpander::BuiltinDerive(expander), mbe::TokenMap::default())))
        }
        MacroDefKind::BuiltInEager(_expander) => None,
    }
}

pub(crate) fn macro_arg(
    db: &dyn AstDatabase,
    id: MacroCallId,
) -> Option<Arc<(tt::Subtree, mbe::TokenMap)>> {
    let id = match id {
        MacroCallId::LazyMacro(id) => id,
        MacroCallId::EagerMacro(_id) => {
            // FIXME: support macro_arg for eager macro
            return None;
        }
    };
    let loc = db.lookup_intern_macro(id);
    let arg = loc.kind.arg(db)?;
    let (tt, tmap) = mbe::syntax_node_to_token_tree(&arg)?;
    Some(Arc::new((tt, tmap)))
}

pub(crate) fn macro_expand(
    db: &dyn AstDatabase,
    id: MacroCallId,
) -> Result<Arc<tt::Subtree>, String> {
    macro_expand_with_arg(db, id, None)
}

// TODO hack
pub fn expander(
    db: &dyn AstDatabase,
    id: MacroCallId,
) -> Option<Arc<(TokenExpander, mbe::TokenMap)>> {
    let lazy_id = match id {
        MacroCallId::LazyMacro(id) => id,
        MacroCallId::EagerMacro(_id) => {
            // TODO
            unimplemented!()
        }
    };

    let loc = db.lookup_intern_macro(lazy_id);
    let macro_rules = db.macro_def(loc.def)?;
    Some(macro_rules)
}

pub(crate) fn macro_expand_with_arg(
    db: &dyn AstDatabase,
    id: MacroCallId,
    arg: Option<Arc<(tt::Subtree, mbe::TokenMap)>>,
) -> Result<Arc<tt::Subtree>, String> {
    let lazy_id = match id {
        MacroCallId::LazyMacro(id) => id,
        MacroCallId::EagerMacro(id) => {
            // TODO
            return Ok(db.lookup_intern_eager_expansion(id).subtree);
        }
    };

    let loc = db.lookup_intern_macro(lazy_id);
    let macro_arg = arg.or_else(|| db.macro_arg(id)).ok_or("Fail to args in to tt::TokenTree")?;

    let macro_rules = db.macro_def(loc.def).ok_or("Fail to find macro definition")?;
    let tt = macro_rules.0.expand(db, lazy_id, &macro_arg.0).map_err(|err| format!("{:?}", err))?;
    // Set a hard limit for the expanded tt
    let count = tt.count();
    if count > 65536 {
        return Err(format!("Total tokens count exceed limit : count = {}", count));
    }
    Ok(Arc::new(tt))
}

pub(crate) fn parse_or_expand(db: &dyn AstDatabase, file_id: HirFileId) -> Option<SyntaxNode> {
    match file_id.0 {
        HirFileIdRepr::FileId(file_id) => Some(db.parse(file_id).tree().syntax().clone()),
        HirFileIdRepr::MacroFile(macro_file) => {
            db.parse_macro(macro_file).map(|(it, _)| it.syntax_node())
        }
    }
}

pub(crate) fn parse_macro(
    db: &dyn AstDatabase,
    macro_file: MacroFile,
) -> Option<(Parse<SyntaxNode>, Arc<mbe::TokenMap>)> {
    parse_macro_with_arg(db, macro_file, None)
}

pub fn parse_macro_with_arg(
    db: &dyn AstDatabase,
    macro_file: MacroFile,
    arg: Option<Arc<(tt::Subtree, mbe::TokenMap)>>,
) -> Option<(Parse<SyntaxNode>, Arc<mbe::TokenMap>)> {
    let _p = profile("parse_macro_query");

    let macro_call_id = macro_file.macro_call_id;
    let expansion = if let Some(arg) = arg {
        macro_expand_with_arg(db, macro_call_id, Some(arg))
    } else {
        db.macro_expand(macro_call_id)
    };
    let tt = expansion
        .map_err(|err| {
            // Note:
            // The final goal we would like to make all parse_macro success,
            // such that the following log will not call anyway.
            match macro_call_id {
                MacroCallId::LazyMacro(id) => {
                    let loc: MacroCallLoc = db.lookup_intern_macro(id);
                    let node = loc.kind.node(db);

                    // collect parent information for warning log
                    let parents = std::iter::successors(loc.kind.file_id().call_node(db), |it| {
                        it.file_id.call_node(db)
                    })
                    .map(|n| format!("{:#}", n.value))
                    .collect::<Vec<_>>()
                    .join("\n");

                    eprintln!(
                        "fail on macro_parse: (reason: {} macro_call: {:#}) parents: {}",
                        err, node.value, parents
                    );
                }
                _ => {
                    eprintln!("fail on macro_parse: (reason: {})", err);
                }
            }
        })
        .ok()?;

    let fragment_kind = to_fragment_kind(db, macro_call_id);

    let (parse, rev_token_map) = mbe::token_tree_to_syntax_node(&tt, fragment_kind).ok()?;
    Some((parse, Arc::new(rev_token_map)))
}

/// Given a `MacroCallId`, return what `FragmentKind` it belongs to.
/// FIXME: Not completed
fn to_fragment_kind(db: &dyn AstDatabase, id: MacroCallId) -> FragmentKind {
    let lazy_id = match id {
        MacroCallId::LazyMacro(id) => id,
        MacroCallId::EagerMacro(id) => {
            return db.lookup_intern_eager_expansion(id).fragment;
        }
    };
    let syn = db.lookup_intern_macro(lazy_id).kind.node(db).value;

    let parent = match syn.parent() {
        Some(it) => it,
        None => {
            // FIXME:
            // If it is root, which means the parent HirFile
            // MacroKindFile must be non-items
            // return expr now.
            return FragmentKind::Expr;
        }
    };

    match parent.kind() {
        MACRO_ITEMS | SOURCE_FILE => FragmentKind::Items,
        ITEM_LIST => FragmentKind::Items,
        LET_STMT => {
            // FIXME: Handle Pattern
            FragmentKind::Expr
        }
        // FIXME: Expand to statements in appropriate positions; HIR lowering needs to handle that
        EXPR_STMT | BLOCK => FragmentKind::Expr,
        ARG_LIST => FragmentKind::Expr,
        TRY_EXPR => FragmentKind::Expr,
        TUPLE_EXPR => FragmentKind::Expr,
        PAREN_EXPR => FragmentKind::Expr,

        FOR_EXPR => FragmentKind::Expr,
        PATH_EXPR => FragmentKind::Expr,
        LAMBDA_EXPR => FragmentKind::Expr,
        CONDITION => FragmentKind::Expr,
        BREAK_EXPR => FragmentKind::Expr,
        RETURN_EXPR => FragmentKind::Expr,
        BLOCK_EXPR => FragmentKind::Expr,
        MATCH_EXPR => FragmentKind::Expr,
        MATCH_ARM => FragmentKind::Expr,
        MATCH_GUARD => FragmentKind::Expr,
        RECORD_FIELD => FragmentKind::Expr,
        CALL_EXPR => FragmentKind::Expr,
        INDEX_EXPR => FragmentKind::Expr,
        METHOD_CALL_EXPR => FragmentKind::Expr,
        AWAIT_EXPR => FragmentKind::Expr,
        CAST_EXPR => FragmentKind::Expr,
        REF_EXPR => FragmentKind::Expr,
        PREFIX_EXPR => FragmentKind::Expr,
        RANGE_EXPR => FragmentKind::Expr,
        BIN_EXPR => FragmentKind::Expr,
        _ => {
            // Unknown , Just guess it is `Items`
            FragmentKind::Items
        }
    }
}
