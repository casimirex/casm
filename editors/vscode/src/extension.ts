/**
 * CASIMIR VS Code client.
 *
 * Deliberately thin: it launches `casm-lsp` and lets the server do everything. Any logic
 * added here would be logic the Neovim, Helix, and Zed users do not get — the point of
 * implementing LSP is that the editor integration stays boring.
 *
 * The one exception is `casm.generateDiagram`, which needs an editor to *show* the result
 * in. The server renders the diagram; the client only decides where it appears.
 */

import { workspace, window, commands, ExtensionContext, ViewColumn } from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

export async function activate(context: ExtensionContext): Promise<void> {
  await start(context);

  context.subscriptions.push(
    commands.registerCommand('casm.restartServer', async () => {
      await stop();
      await start(context);
      window.showInformationMessage('CASIMIR: language server restarted.');
    }),
  );

  context.subscriptions.push(
    commands.registerCommand('casm.generateDiagram', () => generateDiagram()),
  );

  context.subscriptions.push(
    commands.registerCommand('casm.validateWorkspace', () => validateWorkspace()),
  );
}

export async function deactivate(): Promise<void> {
  await stop();
}

async function start(context: ExtensionContext): Promise<void> {
  const configured = workspace.getConfiguration('casm').get<string>('server.path');
  const command = configured && configured.length > 0 ? configured : 'casm-lsp';

  const serverOptions: ServerOptions = {
    run: { command, transport: TransportKind.stdio },
    debug: { command, transport: TransportKind.stdio },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'casm' }],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher('**/*.{yaml,yml}'),
    },
    outputChannelName: 'CASIMIR',
  };

  client = new LanguageClient('casm', 'CASIMIR', serverOptions, clientOptions);

  try {
    await client.start();
    context.subscriptions.push(client);
  } catch (error) {
    // The overwhelmingly common cause is the binary not being installed, so say that
    // rather than surfacing a raw spawn error.
    window.showErrorMessage(
      `CASIMIR: could not start '${command}'. Install it with ` +
        '`cargo install --path crates/casm-lsp`, or set `casm.server.path`. ' +
        `(${error instanceof Error ? error.message : String(error)})`,
    );
    client = undefined;
  }
}

async function stop(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}

/** Asks the server to render the active document, and opens the result beside it. */
async function generateDiagram(): Promise<void> {
  const editor = window.activeTextEditor;
  if (!client || !editor) {
    window.showWarningMessage('CASIMIR: open an architecture file first.');
    return;
  }

  const diagram = await client.sendRequest<string>('workspace/executeCommand', {
    command: 'casm.generateDiagram',
    arguments: [editor.document.uri.toString()],
  });

  const document = await workspace.openTextDocument({
    content: diagram,
    language: 'markdown',
  });
  await window.showTextDocument(document, ViewColumn.Beside);
}

/** Asks the server to summarise findings across every open architecture. */
async function validateWorkspace(): Promise<void> {
  if (!client) {
    window.showWarningMessage('CASIMIR: the language server is not running.');
    return;
  }

  const summary = await client.sendRequest<string>('workspace/executeCommand', {
    command: 'casm.validateWorkspace',
    arguments: [],
  });
  window.showInformationMessage(`CASIMIR: ${summary}`);
}
