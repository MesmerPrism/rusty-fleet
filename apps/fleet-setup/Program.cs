// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

using System.Security.Cryptography;
using System.Text.Json;

namespace RustyFleet.Setup;

internal static class Program
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        WriteIndented = true,
    };

    public static int Main(string[] args)
    {
        try
        {
            if (!OperatingSystem.IsWindows())
            {
                throw new PlatformNotSupportedException("Rusty Fleet Setup requires Windows.");
            }

            var planOnly = args.SequenceEqual(["--plan", "--json"], StringComparer.Ordinal);
            var uninstall = args.SequenceEqual(["--uninstall"], StringComparer.Ordinal);
            var uninstallWorker = args.SequenceEqual(
                ["--uninstall-worker"],
                StringComparer.Ordinal);
            if (!planOnly && !uninstall && !uninstallWorker && args.Length != 0)
            {
                throw new ArgumentException(
                    "accepted arguments are exactly: --plan --json or --uninstall");
            }

            var installRoot = ResolveInstallRoot();
            if (uninstall || uninstallWorker)
            {
                if (uninstall && IsInsideInstallRoot(Environment.ProcessPath, installRoot))
                {
                    StartUninstallWorker();
                    Console.WriteLine(
                        "{\"schema\":\"rusty.fleet.windows_setup_uninstall_handoff.v1\"," +
                        "\"result\":\"started\"}");
                    return 0;
                }
                var uninstallResult = Installer.Uninstall(installRoot);
                Console.WriteLine(JsonSerializer.Serialize(uninstallResult, JsonOptions));
                return 0;
            }

            var bundle = EmbeddedBundle.Load();
            using var runningSetup = OpenRunningSetup();
            var plan = new SetupPlan(
                Schema: "rusty.fleet.guided_installer_plan.v2",
                Product: ReleaseConfiguration.ProductId,
                Version: ReleaseConfiguration.Version,
                Channel: ReleaseConfiguration.Channel,
                AssetSha256: Convert.ToHexStringLower(SHA256.HashData(runningSetup)),
                AuthenticodeTrustMode: ReleaseConfiguration.AuthenticodeTrustMode,
                SignerCertificateSha256: string.IsNullOrEmpty(
                    ReleaseConfiguration.SignerCertificateSha256)
                    ? null
                    : ReleaseConfiguration.SignerCertificateSha256,
                SignerSelfIssued: ReleaseConfiguration.SignerSelfIssued,
                PublicTrustClaim: ReleaseConfiguration.PublicTrustClaim,
                TimestampRequired: ReleaseConfiguration.TimestampRequired,
                Ready: true);

            if (planOnly)
            {
                Console.Out.WriteLine(JsonSerializer.Serialize(plan, JsonOptions));
                return 0;
            }

            Console.WriteLine($"{ReleaseConfiguration.DisplayName} Setup");
            Console.WriteLine($"Version: {plan.Version} ({plan.Channel})");
            Console.WriteLine($"Install location: {installRoot}");
            if (ReleaseConfiguration.BuildKind == "unsigned-dev")
            {
                Console.WriteLine();
                Console.WriteLine("DEVELOPMENT BUILD: this Setup is not a supported public release.");
            }
            else if (!ReleaseConfiguration.PublicTrustClaim)
            {
                Console.WriteLine();
                Console.WriteLine(
                    "LABS SIGNATURE NOTICE: this build is signed by the exact pinned");
                Console.WriteLine(
                    "MesmerPrism certificate, but that certificate is self-issued and");
                Console.WriteLine(
                    "does not have a public Windows trust chain. Windows may show an");
                Console.WriteLine(
                    "Unknown publisher warning. Confirm MesmerPrism only if you chose Labs.");
                Console.WriteLine(
                    "Setup never installs or changes a Windows root certificate.");
            }
            Console.WriteLine();
            Console.WriteLine("Setup changes only the per-user Fleet release pointer. It will not");
            Console.WriteLine("start Fleet, create configuration, invoke ADB, or onboard a headset.");
            Console.WriteLine();
            Console.Write("Choose [I]nstall/update, [R]ollback, or [N]o change: ");
            var answer = Console.ReadLine()?.Trim();
            SetupAction action;
            if (string.Equals(answer, "i", StringComparison.OrdinalIgnoreCase) ||
                string.Equals(answer, "install", StringComparison.OrdinalIgnoreCase) ||
                string.Equals(answer, "update", StringComparison.OrdinalIgnoreCase))
            {
                action = SetupAction.Install;
            }
            else if (string.Equals(answer, "r", StringComparison.OrdinalIgnoreCase) ||
                     string.Equals(answer, "rollback", StringComparison.OrdinalIgnoreCase))
            {
                action = SetupAction.Rollback;
            }
            else
            {
                Console.WriteLine("No changes were made.");
                return 2;
            }

            var result = Installer.Execute(action, bundle, installRoot);
            Console.WriteLine(JsonSerializer.Serialize(result, JsonOptions));
            Console.WriteLine();
            Console.WriteLine(action == SetupAction.Rollback
                ? "Rollback verified. Start Fleet explicitly when ready."
                : "Installation verified. Start Fleet explicitly when ready.");
            return 0;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"Rusty Fleet Setup failed: {ex.Message}");
            return 1;
        }
    }

    private static string ResolveInstallRoot() =>
        string.IsNullOrEmpty(ReleaseConfiguration.DevelopmentInstallRoot)
            ? Path.GetFullPath(Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                ReleaseConfiguration.InstallDirectoryName))
            : ReleaseConfiguration.BuildKind == "unsigned-dev"
                ? Path.GetFullPath(ReleaseConfiguration.DevelopmentInstallRoot)
                : throw new InvalidDataException(
                    "release Setup contains a development install-root override");

    private static bool IsInsideInstallRoot(string? path, string installRoot)
    {
        if (path is null)
        {
            return false;
        }
        var prefix = Path.TrimEndingDirectorySeparator(
            Path.GetFullPath(installRoot)) + Path.DirectorySeparatorChar;
        return Path.GetFullPath(path).StartsWith(
            prefix,
            StringComparison.OrdinalIgnoreCase);
    }

    private static void StartUninstallWorker()
    {
        var runningSetup = Environment.ProcessPath
            ?? throw new IOException("running Setup path is unavailable");
        var workerPath = Path.Combine(
            Path.GetTempPath(),
            $"{ReleaseConfiguration.ProductId}-uninstall-{Guid.NewGuid():N}.exe");
        File.Copy(runningSetup, workerPath, overwrite: false);
        using var process = System.Diagnostics.Process.Start(
            new System.Diagnostics.ProcessStartInfo
            {
                FileName = workerPath,
                Arguments = "--uninstall-worker",
                UseShellExecute = false,
                CreateNoWindow = true,
            }) ?? throw new IOException("could not start isolated uninstall worker");
    }

    private static FileStream OpenRunningSetup()
    {
        var path = Environment.ProcessPath
            ?? throw new IOException("running Setup path is unavailable");
        return new FileStream(
            path,
            FileMode.Open,
            FileAccess.Read,
            FileShare.Read,
            1024 * 1024,
            FileOptions.SequentialScan);
    }

    private sealed record SetupPlan(
        string Schema,
        string Product,
        string Version,
        string Channel,
        string AssetSha256,
        string AuthenticodeTrustMode,
        string? SignerCertificateSha256,
        bool SignerSelfIssued,
        bool PublicTrustClaim,
        bool TimestampRequired,
        bool Ready);
}
