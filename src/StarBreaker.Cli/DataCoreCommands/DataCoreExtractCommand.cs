using System.Diagnostics;
using System.Xml.Linq;
using CliFx;
using CliFx.Attributes;
using CliFx.Infrastructure;
using StarBreaker.Cli.Utils;
using StarBreaker.DataCore;
using StarBreaker.P4k;

namespace StarBreaker.Cli;

[Command("dcb-extract", Description = "Extracts a DataCore binary file into separate xml files")]
public class DataCoreExtractCommand : ICommand
{
    [CommandOption("p4k", 'p', Description = "Path to the Data.p4k", EnvironmentVariable = "INPUT_P4K")]
    public string? P4kFile { get; init; }

    [CommandOption("dcb", 'd', Description = "Path to the Game.dcb", EnvironmentVariable = "INPUT_DCB")]
    public string? DcbFile { get; init; }

    [CommandOption("output", 'o', Description = "Path to the output directory", EnvironmentVariable = "OUTPUT_FOLDER")]
    public required string OutputDirectory { get; init; }

    [CommandOption("filter", 'f', Description = "Pattern to filter entries", EnvironmentVariable = "FILTER")]
    public string? Filter { get; init; }
    
    [CommandOption("text-format", 't', Description = "Output text format", EnvironmentVariable = "TEXT_FORMAT")]
    public string? TextFormat { get; init; }

    public ValueTask ExecuteAsync(IConsole console)
    {
        if (P4kFile == null && DcbFile == null)
        {
            console.Output.WriteLine("P4k or DCB files are required.");
            return default;
        }

        if (!string.IsNullOrEmpty(P4kFile) && !string.IsNullOrEmpty(DcbFile))
        {
            console.Output.WriteLine("Only one of P4k and DCB files can be specified.");
            return default;
        }

        Stream? dcbStream = null;
        if (!string.IsNullOrEmpty(DcbFile))
        {
            dcbStream = File.OpenRead(DcbFile);
            console.Output.WriteLine("DCB loaded.");
        }
        else if (!string.IsNullOrEmpty(P4kFile))
        {
            var p4k = P4kDirectoryNode.FromP4k(P4k.P4kFile.FromFile(P4kFile));
            console.Output.WriteLine("P4k loaded.");
            foreach (var file in DataCoreUtils.KnownPaths)
            {
                if (!p4k.FileExists(file)) continue;

                dcbStream = p4k.OpenRead(file);
                console.Output.WriteLine($"{file} found");
                break;
            }
        }

        if (dcbStream == null)
        {
            console.Output.WriteLine("DataCore not found.");
            return default;
        }
        
        var df = TextFormat switch
        {   
            "json" => DataForge.FromDcbStreamJson(dcbStream),
            _ => DataForge.FromDcbStreamXml(dcbStream),
        };

        console.Output.WriteLine("DataCore loaded.");
        console.Output.WriteLine($"Exporting as {TextFormat ?? "xml"} to {OutputDirectory}...");

        var sw = Stopwatch.StartNew();
        df.ExtractAllParallel(OutputDirectory, Filter, new ProgressBar(console));
        sw.Stop();

        console.Output.WriteLine();
        console.Output.WriteLine($"Export completed in {sw.ElapsedMilliseconds}ms.");

        return default;
    }
}