#![feature(box_patterns)]

use std::sync::Arc;

use deps_analyzer::DepsAnalyzer;
use farmfe_core::{
  config::Config,
  context::CompilationContext,
  error::{CompilationError, Result},
  module::{Module, ModuleId, ModuleMetaData, ModuleSystem, ScriptModuleMetaData},
  plugin::{
    Plugin, PluginAnalyzeDepsHookParam, PluginHookContext, PluginLoadHookParam,
    PluginLoadHookResult, PluginParseHookParam, PluginProcessModuleHookParam,
  },
  resource::{
    resource_pot::{ResourcePot, ResourcePotType},
    Resource, ResourceType,
  },
  swc_common::{comments::NoopComments, Mark, GLOBALS},
  swc_ecma_ast::ModuleItem,
};
use farmfe_toolkit::{
  fs::read_file_utf8,
  script::{codegen_module, module_type_from_id, parse_module, syntax_from_module_type},
  swc_ecma_transforms::{
    react::{react, Options},
    resolver,
    typescript::{strip, strip_with_jsx},
  },
  swc_ecma_visit::VisitMutWith,
};

mod deps_analyzer;
/// ScriptPlugin is used to support compiling js/ts/jsx/tsx/... files, support loading, parse, analyze dependencies and code generation.
/// Note that we do not do transforms here, the transforms (e.g. strip types, jsx...) are handled in a separate plugin (farmfe_plugin_swc_transforms).
pub struct FarmPluginScript {}

impl Plugin for FarmPluginScript {
  fn name(&self) -> &str {
    "FarmPluginScript"
  }

  fn load(
    &self,
    param: &PluginLoadHookParam,
    _context: &Arc<CompilationContext>,
    _hook_context: &PluginHookContext,
  ) -> Result<Option<PluginLoadHookResult>> {
    let module_type = module_type_from_id(param.resolved_path);

    if module_type.is_script() {
      let content = read_file_utf8(param.resolved_path)?;

      Ok(Some(PluginLoadHookResult {
        content,
        module_type,
      }))
    } else {
      Ok(None)
    }
  }

  fn parse(
    &self,
    param: &PluginParseHookParam,
    context: &Arc<CompilationContext>,
    _hook_context: &PluginHookContext,
  ) -> Result<Option<ModuleMetaData>> {
    if let Some(syntax) = syntax_from_module_type(&param.module_type) {
      let mut swc_module = parse_module(
        &param.module_id.to_string(),
        &param.content,
        syntax.clone(),
        context.meta.script.cm.clone(),
      )?;

      GLOBALS.set(&context.meta.script.globals, || {
        let top_level_mark = Mark::new();
        let unresolved_mark = Mark::new();

        swc_module.visit_mut_with(&mut resolver(
          unresolved_mark,
          top_level_mark,
          param.module_type.is_typescript(),
        ));

        let module_system = if swc_module
          .body
          .iter()
          .any(|item| matches!(item, ModuleItem::ModuleDecl(_)))
        {
          ModuleSystem::EsModule
        } else {
          ModuleSystem::CommonJs
        };

        let meta = ScriptModuleMetaData {
          ast: swc_module,
          top_level_mark: top_level_mark.as_u32(),
          unresolved_mark: unresolved_mark.as_u32(),
          module_system,
        };

        Ok(Some(ModuleMetaData::Script(meta)))
      })
    } else {
      Ok(None)
    }
  }

  fn analyze_deps(
    &self,
    param: &mut PluginAnalyzeDepsHookParam,
    context: &Arc<CompilationContext>,
  ) -> Result<Option<()>> {
    let module = param.module;

    if module.module_type.is_script() {
      let module_ast = &module.meta.as_script().ast;
      let mut analyzer = DepsAnalyzer::new(
        module_ast,
        Mark::from_u32(module.meta.as_script().unresolved_mark),
      );

      GLOBALS.set(&context.meta.script.globals, || {
        let deps = analyzer.analyze_deps();
        param.deps.extend(deps);
      });

      Ok(Some(()))
    } else {
      Ok(None)
    }
  }

  fn process_module(
    &self,
    param: &mut PluginProcessModuleHookParam,
    context: &Arc<CompilationContext>,
  ) -> Result<Option<()>> {
    if param.module_type.is_typescript() {
      GLOBALS.set(&context.meta.script.globals, || {
        let top_level_mark = Mark::from_u32(param.meta.as_script().top_level_mark);
        let ast = &mut param.meta.as_script_mut().ast;

        match param.module_type {
          farmfe_core::module::ModuleType::Js => {
            // TODO downgrade syntax
          }
          farmfe_core::module::ModuleType::Jsx => {
            ast.visit_mut_with(&mut react(
              context.meta.script.cm.clone(),
              Some(NoopComments), // TODO parse comments
              Options::default(),
              top_level_mark,
            ));
          }
          farmfe_core::module::ModuleType::Ts => {
            ast.visit_mut_with(&mut strip(top_level_mark));
          }
          farmfe_core::module::ModuleType::Tsx => {
            ast.visit_mut_with(&mut strip_with_jsx(
              context.meta.script.cm.clone(),
              Default::default(),
              NoopComments, // TODO parse comments
              top_level_mark,
            ));
            ast.visit_mut_with(&mut react(
              context.meta.script.cm.clone(),
              Some(NoopComments), // TODO parse comments
              Options::default(),
              top_level_mark,
            ));
          }
          _ => {}
        }
      });
    }

    Ok(Some(()))
  }

  fn generate_resources(
    &self,
    resource_pot: &mut ResourcePot,
    context: &Arc<CompilationContext>,
    _hook_context: &PluginHookContext,
  ) -> Result<Option<Vec<Resource>>> {
    if matches!(resource_pot.resource_pot_type, ResourcePotType::Js) {
      let ast = &resource_pot.meta.as_js().ast;
      let buf = codegen_module(ast, context.meta.script.cm.clone()).map_err(|e| {
        CompilationError::GenerateResourcesError {
          name: resource_pot.id.to_string(),
          ty: resource_pot.resource_pot_type.clone(),
          source: Some(Box::new(e)),
        }
      })?;

      Ok(Some(vec![Resource {
        bytes: buf,
        name: resource_pot.id.to_string().replace("../", "") + ".js", // TODO generate file name based on config
        emitted: false,
        resource_type: ResourceType::Js,
        resource_pot: resource_pot.id.clone(),
      }]))
    } else {
      Ok(None)
    }
  }
}

impl FarmPluginScript {
  pub fn new(_config: &Config) -> Self {
    Self {}
  }
}