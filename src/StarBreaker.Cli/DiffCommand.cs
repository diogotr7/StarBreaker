using System.Diagnostics;
using CliFx;
using CliFx.Attributes;
using CliFx.Infrastructure;
using StarBreaker.DataCore;
using StarBreaker.P4k;
using ZstdSharp;

namespace StarBreaker.Cli;

[Command("diff", Description = "Dumps game information into plain text files for comparison")]
public class DiffCommand : ICommand
{
    [CommandOption("game", 'g', Description = "Path to the game folder", EnvironmentVariable = "GAME_FOLDER")]
    public required string GameFolder { get; init; }

    [CommandOption("output", 'o', Description = "Path to the output directory", EnvironmentVariable = "OUTPUT_FOLDER")]
    public required string OutputDirectory { get; init; }

    [CommandOption("keep", 'k', Description = "Keep old files in the output directory", EnvironmentVariable = "KEEP_OLD")]
    public bool KeepOld { get; init; }

    [CommandOption("format", 'f', Description = "Output format", EnvironmentVariable = "TEXT_FORMAT")]
    public string TextFormat { get; init; } = "xml";

    public async ValueTask ExecuteAsync(IConsole console)
    {
        if (!KeepOld)
        {
            await console.Output.WriteLineAsync("Deleting old files...");
            string[] deleteFolder =
            [
                Path.Combine(OutputDirectory, "DataCore"),
                Path.Combine(OutputDirectory, "P4k"),
                Path.Combine(OutputDirectory, "P4kContents"),
                Path.Combine(OutputDirectory, "Localization"),
                Path.Combine(OutputDirectory, "build_manifest.json"),
                Path.Combine(OutputDirectory, "DataCore.dcb.zst"),
                Path.Combine(OutputDirectory, "StarCitizen.exe.zst"),
            ];
            string[] deleteFile =
            [
                Path.Combine(OutputDirectory, "build_manifest.json"),
            ];

            foreach (var folder in deleteFolder)
            {
                if (Directory.Exists(folder))
                    Directory.Delete(folder, true);
            }

            foreach (var file in deleteFile)
            {
                if (File.Exists(file))
                    File.Delete(file);
            }

            await console.Output.WriteLineAsync("Old files deleted.");
        }

        // Hide output from subcommands
        var fakeConsole = new FakeConsole();

        var p4kFile = Path.Combine(GameFolder, "Data.p4k");
        var exeFile = Path.Combine(GameFolder, "Bin64", "StarCitizen.exe");

        var dumpP4k = new DumpP4kCommand
        {
            P4kFile = p4kFile,
            OutputDirectory = Path.Combine(OutputDirectory, "P4k")
        };
        await dumpP4k.ExecuteAsync(fakeConsole);
        await console.Output.WriteLineAsync("P4k dumped.");

        var localizationDump = new ExtractP4kCommand
        {
            P4kFile = p4kFile,
            OutputDirectory = Path.Combine(OutputDirectory, "P4kContents"),
            FilterPattern = "*english\\global.ini",
        };
        await localizationDump.ExecuteAsync(fakeConsole);
        await console.Output.WriteLineAsync("Localization dumped.");

        var dcbExtract = new DataCoreExtractCommand
        {
            P4kFile = p4kFile,
            OutputDirectory = Path.Combine(OutputDirectory, "DataCore"),
            TextFormat = TextFormat
        };
        await dcbExtract.ExecuteAsync(fakeConsole);
        await console.Output.WriteLineAsync("DataCore extracted.");

        var extractProtobufs = new ExtractProtobufsCommand
        {
            Input = exeFile,
            Output = Path.Combine(OutputDirectory, "Protobuf")
        };
        await extractProtobufs.ExecuteAsync(fakeConsole);
        await console.Output.WriteLineAsync("Protobufs extracted.");

        var extractDescriptor = new ExtractDescriptorSetCommand
        {
            Input = exeFile,
            Output = Path.Combine(OutputDirectory, "Protobuf", "descriptor_set.bin")
        };
        await extractDescriptor.ExecuteAsync(fakeConsole);
        await console.Output.WriteLineAsync("Descriptor set extracted.");

        await ExtractDataCoreIntoZip(p4kFile, Path.Combine(OutputDirectory, "DataCore.dcb.zst"));
        await ExtractExecutableIntoZip(exeFile, Path.Combine(OutputDirectory, "StarCitizen.exe.zst"));
        File.Copy(Path.Combine(GameFolder, "build_manifest.id"), Path.Combine(OutputDirectory, "build_manifest.json"), true);
        await console.Output.WriteLineAsync("Zipped DataCore and StarCitizen.");

        await console.Output.WriteLineAsync("Done.");
    }

    private static async Task ExtractDataCoreIntoZip(string p4kFile, string zipPath)
    {
        var p4k = P4kDirectoryNode.FromP4k(P4kFile.FromFile(p4kFile));
        Stream? input = null;
        foreach (var file in DataCoreUtils.KnownPaths)
        {
            if (!p4k.FileExists(file)) continue;

            input = p4k.OpenRead(file);
            break;
        }

        if (input == null)
            throw new InvalidOperationException("DataCore not found.");

        await using var output = File.OpenWrite(zipPath);
        await using var compressionStream = new CompressionStream(output, leaveOpen: false);
        await input.CopyToAsync(compressionStream);
    }

    private static async Task ExtractExecutableIntoZip(string exeFile, string zipPath)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(zipPath)!);
        await using var input = File.OpenRead(exeFile);
        await using var output = File.OpenWrite(zipPath);

        await using var compressionStream = new CompressionStream(output, leaveOpen: false);
        await input.CopyToAsync(compressionStream);
    }
}