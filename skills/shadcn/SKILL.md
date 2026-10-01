---
name: shadcn
description: shadcn/ui — agregar, buscar y componer componentes, themear con variables CSS, presets y registries. Se carga cuando el proyecto tiene components.json.
---

# shadcn/ui

Los componentes de shadcn se copian al proyecto como código fuente, con el CLI. Nada de dependencia de UI: el `package.json` solo suma las libs de base (primitivas, iconos, cva).

Los archivos de esta skill viven en `/usr/local/share/jimmy/skills/shadcn/` y son la referencia upstream (en inglés) que corresponde al CLI:

- `cli.md` — todos los comandos y flags, presets y templates.
- `customization.md` — theming, variables CSS, OKLCH, dark mode, radio, variantes.
- `registry.md` — autoría de registries propios.
- `mcp.md` — el MCP server de shadcn.
- `rules/styling.md`, `rules/forms.md`, `rules/composition.md`, `rules/icons.md`, `rules/base-vs-radix.md`, `rules/chat.md` — las reglas duras, con pares incorrecto/correcto. Leelas antes de escribir el primer componente: son la diferencia entre shadcn y Tailwind suelto.

## Contexto del proyecto

Antes de tocar nada, corré el CLI con el runner del proyecto (`bunx --bun`, `pnpm dlx` o `npx`) y leé el JSON:

```bash
bunx --bun shadcn@latest info --json
```

De ahí salen `framework`, `tailwindVersion`, `tailwindCss`, `base` (`base`, `radix` o `aria`), `iconLibrary`, `aliases`, `resolvedPaths` y los componentes ya instalados. No adivines ninguno de esos campos.

## Reglas duras

- **Componente que existe, componente que se usa.** Antes de escribir un `div` estilizado, `bunx --bun shadcn@latest search` y `view`.
- **Componer, no reinventar.** Ajustes = Tabs + Card; dashboard = Sidebar + Card + Table.
- **Variantes antes que clases.** `variant="outline"`, `size="sm"`.
- **Colores semánticos.** `bg-primary`, `text-muted-foreground`; nunca `bg-blue-500` crudo.
- **`className` es para layout, no para pintar.**
- **Nada de `space-x-*` ni `space-y-*`**: `flex` con `gap-*` (`flex flex-col gap-*` para apilar).
- **`size-10`, no `w-10 h-10`.** `truncate`, no las tres clases a mano. `cn()` para lo condicional.
- **Formularios con `FieldGroup` + `Field`**, y `InputGroup` + `InputGroupInput` para inputs con adornos.
- **Conjuntos de 2 a 7 opciones: `ToggleGroup`**, no un loop de `Button` con estado a mano.
- **Items siempre dentro de su Group**: `SelectItem` → `SelectGroup`, `DropdownMenuItem` → `DropdownMenuGroup`, `CommandItem` → `CommandGroup`.
- **Dialog, Sheet y Drawer siempre llevan Title** (`sr-only` si no se ve).
- **Vacío usa `Empty`, aviso usa `Alert`, `Badge` antes que un `span` pintado, `Separator` antes que un `<hr>`, `Skeleton` antes que un `animate-pulse`.**
- **Iconos**: `data-icon="inline-start"` dentro de un `Button`, sin clases de tamaño, y se pasan como objeto (`icon={CheckIcon}`), no por nombre.
- **Chat**: `MessageScroller` + `Message` + `Bubble`; no hay burbujas hechas a mano ni `useStickToBottom`.
- **`asChild` (radix) o `render` (base)** para triggers propios: mirá `base` en el `info`.
- **Nunca decodifiques un preset a mano**: `shadcn preset decode <code>`, `preset url`, `preset apply`.

## Flujo

1. `info --json` — framework, base, aliases, qué hay instalado.
2. Mirá los componentes instalados antes de `add`: no importes lo que no está ni reinstales lo que ya está.
3. `search` para encontrar, `docs <componente>` para sacar las URLs de docs y ejemplos, y **fetcheá esas URLs** antes de escribir código. `view @shadcn/button` para lo que todavía no instalaste.
4. `add` para instalar. Para actualizar algo ya tocado: `add <x> --dry-run`, después `add <x> --diff <archivo>` y mergeá a mano. `--overwrite` solo con permiso explícito de Don Berti.
5. Después de agregar de un registry de terceros, **leé lo que se agregó**: imports con el alias equivocado, sub-componentes faltantes, iconos de otra librería.
6. Sin registry explícito, preguntá cuál (`@shadcn`, `@tailark`, `owner/repo`).

## Comandos

```bash
bunx --bun shadcn@latest init --base base --template vite --preset nova
bunx --bun shadcn@latest apply a2r6bw --only theme,font
bunx --bun shadcn@latest add button card dialog
bunx --bun shadcn@latest add --all
bunx --bun shadcn@latest add button --dry-run
bunx --bun shadcn@latest add button --diff button.tsx
bunx --bun shadcn@latest search @shadcn -q "sidebar"
bunx --bun shadcn@latest docs button dialog select
bunx --bun shadcn@latest view @shadcn/button
```

Presets con nombre: `nova`, `vega`, `maia`, `lyra`, `mira`, `luma`, `sera`, `rhea`. Templates: `next`, `vite`, `start`, `react-router`, `astro` y `laravel`.

El `init` es interactivo (pregunta el preset): siempre pasale `--preset <nombre>` y `--yes`, y redirigí la entrada con `</dev/null` en el contenedor, o se cuelga esperando una respuesta que nunca llega.
