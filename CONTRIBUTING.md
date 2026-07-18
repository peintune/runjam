# Contributing to RunJam

Thanks for your interest in contributing to RunJam! 🎉

## Getting Started

### Prerequisites

- **Node.js** ≥ 18
- **Rust** ≥ 1.80 (with `cargo`)
- **System dependencies** for Tauri 2:
  - **macOS**: Xcode Command Line Tools
  - **Windows**: Microsoft Visual Studio C++ Build Tools + WebView2

### Setup

```bash
# Clone the repo
git clone https://github.com/nicepkg/runjam.git
cd runjam

# Install frontend dependencies
npm install

# Start dev mode (opens desktop window with hot reload)
npm run tauri dev
```

### Build

```bash
npm run tauri build
```

Build artifacts are in `src-tauri/target/release/bundle/`.

## Development Workflow

1. **Fork** the repository and create your branch from `main`.
2. **Write code** following the existing style:
   - Frontend: Vue 3 `<script setup>` + TypeScript + Tailwind CSS
   - Backend: Rust with `rjlog!` for logging
3. **Test** your changes with `npm run tauri dev`.
4. **Commit** with clear, descriptive messages.
5. **Open a Pull Request** describing what you changed and why.

## Code Style

### Frontend (Vue + TypeScript)

- Use `<script setup lang="ts">` composition API
- Use Tailwind CSS utility classes for styling
- Use Lucide icons from `lucide-vue-next`
- Keep components focused and small

### Backend (Rust)

- Follow standard Rust formatting (`cargo fmt`)
- Use `rjlog!()` macro for logging (not `println!`)
- Handle errors gracefully with `Result` types
- Add Tauri commands in `commands/` and business logic in dedicated modules

## Reporting Bugs

Open a [GitHub Issue](https://github.com/nicepkg/runjam/issues) with:

1. **OS** (macOS / Windows) and version
2. **RunJam version** (find in Settings → General)
3. **Steps to reproduce**
4. **Expected vs actual behavior**
5. **Logs** (from `~/.runjam/logs/`)

## Feature Requests

Have an idea? Open a GitHub Issue with the `enhancement` label. Describe:

- What problem does it solve?
- How would it work from the user's perspective?
- Any alternatives you've considered?

## Project Structure

```
runjam/
├── src-tauri/          # Rust backend (Tauri 2)
│   └── src/
│       ├── commands/   # Tauri command handlers
│       ├── models/     # Data structures
│       ├── agent/      # Agent detection/installation
│       ├── session/    # Session management
│       └── ...
├── src/                # Vue 3 frontend
│   ├── components/     # Reusable UI components
│   ├── views/          # Page views
│   ├── stores/         # Pinia state management
│   ├── api/            # Tauri invoke wrappers
│   └── composables/    # Vue composables
└── landing.html        # Landing page (separate build)
```

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
