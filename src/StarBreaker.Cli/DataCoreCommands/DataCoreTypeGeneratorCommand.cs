﻿using CliFx;
using CliFx.Attributes;
using CliFx.Infrastructure;
using StarBreaker.Common;
using StarBreaker.DataCore;
using StarBreaker.DataCore.TypeGenerator;
using StarBreaker.P4k;

namespace StarBreaker.Cli;

[Command("dcb-generate", Description = "Generates C# types for DataCore structs and enums. Allows typesafe access to DataCore records.")]
public class DataCoreTypeGeneratorCommand : ICommand
{
    [CommandOption("p4k", 'p', Description = "Path to the Data.p4k")]
    public string? P4kFile { get; init; }

    [CommandOption("dcb", 'd', Description = "Path to the Game.dcb")]
    public string? DcbFile { get; init; }

    [CommandOption("output", 'o', Description = "Path to the output directory")]
    public required string OutputDirectory { get; init; }
    
    public ValueTask ExecuteAsync(IConsole console)
    {
        if (P4kFile == null && DcbFile == null)
        {
            console.Output.WriteLine("P4k and DCB files are required.");
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
            var p4k = new P4kFileSystem(P4k.P4kFile.FromFile(P4kFile));
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
        

        console.Output.WriteLine("Generating DataCore types...");
        var dcr = new DataCoreTypeGenerator(new DataCoreDatabase(dcbStream));

        console.Output.WriteLine("Writing DataCore types...");
        dcr.Generate(OutputDirectory);
        
        console.Output.WriteLine("Done.");

        return default;
    }
}