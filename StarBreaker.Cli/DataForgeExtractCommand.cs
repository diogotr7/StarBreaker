using System.Diagnostics;
using System.Text.RegularExpressions;
using CliFx;
using CliFx.Attributes;
using CliFx.Infrastructure;
using StarBreaker.Forge;

namespace StarBreaker.Cli;

[Command("df-extract", Description = "Extracts a DataForge binary file into separate xml files")]
public class DataForgeExtractCommand : ICommand
{
    [CommandOption("dcb", 'd', Description = "Path to the DataForge binary file")]
    public required string DataForgeBinary { get; init; }
    
    [CommandOption("output", 'o', Description = "Path to the output directory")]
    public required string OutputDirectory { get; init; }
    
    [CommandOption("filter", 'f', Description = "Regex pattern to filter entries")]
    public Regex? RegexPattern { get; init; }
    
    public ValueTask ExecuteAsync(IConsole console)
    {
        var dataForge = new DataForge(DataForgeBinary);

        console.Output.WriteLine("DataForge loaded.");
        console.Output.WriteLine("Exporting...");
        
        var sw = Stopwatch.StartNew();
        dataForge.Extract(OutputDirectory, RegexPattern, new ProgressBar(console));
        sw.Stop();
        
        console.Output.WriteLine();
        console.Output.WriteLine($"Export completed in {sw.ElapsedMilliseconds}ms.");
        
        return default;
    }
}