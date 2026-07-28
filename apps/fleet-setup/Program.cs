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
            if (!planOnly && args.Length != 0)
            {
                throw new ArgumentException("accepted arguments are exactly: --plan --json");
            }

            var bundle = EmbeddedBundle.Load();
            var installRoot = ResolveInstallRoot();
            using var runningSetup = OpenRunningSetup();
            var plan = new SetupPlan(
                Schema: "rusty.fleet.guided_installer_plan.v1",
                Product: "rusty-fleet",
                Version: ReleaseConfiguration.Version,
                Channel: ReleaseConfiguration.Channel,
                AssetSha256: Convert.ToHexStringLower(SHA256.HashData(runningSetup)),
                Ready: true);

            if (planOnly)
            {
                Console.Out.WriteLine(JsonSerializer.Serialize(plan, JsonOptions));
                return 0;
            }

            Console.WriteLine("Rusty Fleet Setup");
            Console.WriteLine($"Version: {plan.Version} ({plan.Channel})");
            Console.WriteLine($"Install location: {installRoot}");
            if (ReleaseConfiguration.BuildKind == "unsigned-dev")
            {
                Console.WriteLine();
                Console.WriteLine("DEVELOPMENT BUILD: this Setup is not a supported public release.");
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
                "RustyFleet"))
            : ReleaseConfiguration.BuildKind == "unsigned-dev"
                ? Path.GetFullPath(ReleaseConfiguration.DevelopmentInstallRoot)
                : throw new InvalidDataException(
                    "release Setup contains a development install-root override");

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
        bool Ready);
}
