import { execFile } from "node:child_process";
import { homedir } from "node:os";

function exec(cmd: string, args: string[]): Promise<{ stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    execFile(cmd, args, (err, stdout, stderr) => {
      if (err) reject(err);
      else resolve({ stdout: stdout as string, stderr: stderr as string });
    });
  });
}

const CONTAINER_NAME = "ccd-shared";
const IMAGE_NAME = "ccd-sandbox:latest";

export interface ContainerOptions {
  projectRoots: string[];
  dataDir: string;
  ccdInstallDir: string;
  extraMounts: string[];
}

export class ContainerManager {
  async isRunning(): Promise<boolean> {
    const { stdout } = await exec("docker", ["ps", "-q", "-f", `name=${CONTAINER_NAME}`]);
    return stdout.trim().length > 0;
  }

  async ensureRunning(opts: ContainerOptions): Promise<void> {
    if (await this.isRunning()) return;

    const home = homedir();
    const args = [
      "run", "-d",
      "--name", CONTAINER_NAME,
      "--restart", "unless-stopped",
      "--label", "ccd=shared",
      "--add-host", "host.docker.internal:host-gateway",
    ];

    for (const root of opts.projectRoots) {
      args.push("-v", `${root}:${root}`);
    }

    args.push("-v", `${home}/.claude:${home}/.claude`);
    args.push("-v", `${opts.dataDir}:${opts.dataDir}`);
    args.push("-v", `${opts.ccdInstallDir}:${opts.ccdInstallDir}:ro`);

    for (const mount of opts.extraMounts) {
      args.push("-v", mount);
    }

    args.push(IMAGE_NAME, "tail", "-f", "/dev/null");

    await exec("docker", args);
  }

  async destroy(): Promise<void> {
    try {
      await exec("docker", ["rm", "-f", CONTAINER_NAME]);
    } catch {
      // Container might not exist
    }
  }
}
