<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";
  import type { DaemonStatus, Settings } from "$lib/generated/core";
  let {
    draft = $bindable(),
    status,
    copyCommand,
    fileTool = $bindable("grep"),
  }: {
    draft: Settings;
    status: DaemonStatus;
    copyCommand: (command: string) => void;
    fileTool: string;
  } = $props();
  const fileCommands = [
    {
      tool: "grep",
      purpose: "Search Images",
      shell: "Bash / Zsh",
      prompt: "$",
      title: "Find Matching Images",
      command: 'grep -a -r -l -i -F -- "coffee" "/path/to/images"',
      description:
        "Find files containing “coffee”, including subfolders. Replace the search words and folder path.",
      hint: "The -a option includes binary files. Matches can come from image data, metadata, or appended text.",
    },
    {
      tool: "cat",
      purpose: "Read a File",
      shell: "Bash / Zsh",
      prompt: "$",
      title: "Read the Whole File",
      command: 'cat "/path/to/image.png"',
      description: "Print the image file and its appended text. Replace the path with your image.",
      hint: "Includes binary image data and Lenscribe markers. Use the API below for only the extracted text.",
    },
    {
      tool: "Get-Content",
      purpose: "Read in PowerShell",
      shell: "PowerShell",
      prompt: "PS>",
      title: "Read in PowerShell",
      command: "Get-Content -LiteralPath 'C:/path/to/image.png' -Encoding utf8",
      description:
        "Read the file as UTF-8 to preserve extracted characters. Replace the path with your image.",
      hint: "Includes image data and Lenscribe markers. In PowerShell, cat is an alias for Get-Content.",
    },
  ];
  const curlCommand = $derived(
    'curl -G --data-urlencode "q=coffee" "' +
      (status?.apiUrl ?? "http://127.0.0.1:47831") +
      '/search"',
  );
  const textCommand = $derived(
    'curl "' + (status?.apiUrl ?? "http://127.0.0.1:47831") + '/files/1/text"',
  );
  const fileExample = $derived(fileCommands.find((entry) => entry.tool === fileTool)!);
</script>

<section class="card file-tools-card" aria-labelledby="file-tools-title">
  <div class="file-tools-heading">
    <div class="file-tools-intro">
      <span class="file-tools-icon"><Icon name="terminal" size={22} /></span>
      <div>
        <h2 id="file-tools-title">Use Your Files Directly</h2>
        <p>Text stays in your images, even when Lenscribe is closed.</p>
      </div>
    </div>
  </div>
  <div class="file-tool-picker" role="group" aria-label="File command examples">
    {#each fileCommands as example}
      <button
        type="button"
        class:active={fileTool === example.tool}
        aria-label={example.tool}
        aria-pressed={fileTool === example.tool}
        onclick={() => (fileTool = example.tool)}
        ><span class="file-tool-name">{example.tool}</span><span class="file-tool-purpose"
          >{example.purpose}</span
        ></button
      >
    {/each}
  </div>
  <div class="file-command-window">
    <div class="file-command-toolbar">
      <span><Icon name="terminal" size={15} /> {fileExample.shell}</span><button
        type="button"
        aria-label={"Copy " + fileExample.tool + " command"}
        onclick={() => copyCommand(fileExample.command)}
        ><Icon name="copy" size={14} /> Copy Command</button
      >
    </div>
    <div class="file-command-code">
      <span class="file-command-prompt" aria-hidden="true">{fileExample.prompt}</span>
      <pre><code>{fileExample.command}</code></pre>
    </div>
  </div>
  <div class="file-command-details">
    <h3>{fileExample.title}</h3>
    <p>{fileExample.description}</p>
  </div>
  <div class="file-command-note">
    <Icon name="image" size={16} />
    <p>{fileExample.hint}</p>
  </div>
</section>
<section class="card">
  <label class="switch setting-toggle"
    ><span
      ><strong>Local search API</strong><span
        >Search images and read their text using curl or your own tools.</span
      ></span
    ><input type="checkbox" bind:checked={draft.api.enabled} /><span class="switch-track"
    ></span></label
  >
</section>
<section class="card">
  <h2>Connection</h2>
  <label class="field port-field" for="port"
    >Port<input
      id="port"
      type="number"
      min="0"
      max="65535"
      step="1"
      bind:value={draft.api.port}
    /><span class="hint">Set to 0 to pick an available port.</span></label
  >
  <div class="connection-row">
    <span class="dot" class:quiet={!status.apiUrl}></span><strong
      >{status.apiUrl ? "Listening" : "API is off"}</strong
    >{#if status.apiUrl}<code>{status.apiUrl}</code>{/if}
  </div>
  <p class="hint">
    Runs on this computer at 127.0.0.1. It searches your local index by filename and extracted text.
  </p>
</section>
<section class="card command-card">
  <div class="section-heading">
    <h2>Search the index with curl</h2>
    <button
      type="button"
      aria-label="Copy curl search command"
      onclick={() => copyCommand(curlCommand)}
      disabled={!status.apiUrl}><Icon name="copy" size={15} /> Copy command</button
    >
  </div>
  <p class="hint">
    Replace “coffee” with the words you want to find. In Windows PowerShell, use curl.exe.
  </p>
  <pre><code>{curlCommand}</code></pre>
  {#if !status.apiUrl}<p class="hint">
      Enable the API and save settings to use these commands.
    </p>{/if}
  <div class="section-heading command-heading">
    <h3>Read only the extracted text</h3>
    <button
      type="button"
      aria-label="Copy curl text command"
      onclick={() => copyCommand(textCommand)}
      disabled={!status.apiUrl}><Icon name="copy" size={15} /> Copy command</button
    >
  </div>
  <p class="hint">Replace 1 with a file ID from the search results.</p>
  <pre><code>{textCommand}</code></pre>
  <div class="endpoint-list">
    <div><code>GET /search?q=…</code><span>Find matching images</span></div>
    <div><code>GET /files/:id/text</code><span>Read extracted text</span></div>
    <div><code>GET /folders</code><span>List indexed folders</span></div>
    <div><code>GET /health</code><span>Check the API</span></div>
  </div>
</section>
