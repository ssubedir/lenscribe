export type Page = "overview" | "folders" | "extraction" | "api" | "general";

export const pages: { id: Page; label: string; icon: string; description: string }[] = [
  {
    id: "overview",
    label: "Overview",
    icon: "overview",
    description: "Your images, becoming searchable.",
  },
  {
    id: "folders",
    label: "Watched Folders",
    icon: "folder",
    description: "Choose where Lenscribe looks for images.",
  },
  {
    id: "extraction",
    label: "AI Extraction",
    icon: "spark",
    description: "Connect the vision model that reads your images.",
  },
  {
    id: "api",
    label: "Search & Read",
    icon: "terminal",
    description: "Use your usual file tools, or query the local search API.",
  },
  {
    id: "general",
    label: "General",
    icon: "settings",
    description: "Make Lenscribe fit your workflow.",
  },
];

export function folderName(path: string) {
  return (
    path
      .replace(/[\\/]+$/, "")
      .split(/[\\/]/)
      .pop() || "New folder"
  );
}

export type Inspection = { folderId: number; name: string; enabled: boolean; initialPath?: string };
