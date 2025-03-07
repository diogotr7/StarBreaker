﻿using CliFx;
using CliFx.Attributes;
using CliFx.Infrastructure;
using StarBreaker.Protobuf;

namespace StarBreaker.Cli;

[Command("proto-extract", Description = "Extracts protobuf definitions from the Star Citizen executable.")]
public class ExtractProtobufsCommand : ICommand
{
    [CommandOption("input", 'i', Description = "The path to the Star Citizen executable.", EnvironmentVariable = "INPUT_FILE")]
    public required string Input { get; init; }

    [CommandOption("output", 'o', Description = "The path to the output directory.", EnvironmentVariable = "OUTPUT_FOLDER")]
    public required string Output { get; init; }

    public ValueTask ExecuteAsync(IConsole console)
    {
        console.Output.WriteLine("Extracting protobuf definitions...");
        var extractor = ProtobufExtractor.FromFilename(Input);
        extractor.WriteProtos(Output, p => !p.Name.StartsWith("google/protobuf"));
        console.Output.WriteLine("Wrote {0} protobuf definitions to {1}", extractor.DescriptorSet.File.Count, Output);

        return default;
    }
}