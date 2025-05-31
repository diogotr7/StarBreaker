using System.Globalization;
using System.Xml.Linq;
using CliFx;
using CliFx.Attributes;
using CliFx.Infrastructure;
using StarBreaker.P4k;

namespace StarBreaker.Cli;

[Command("p4k-dump", Description = "Dumps the contents a Game.p4k file")]
public class DumpP4kCommand : ICommand
{
    [CommandOption("p4k", 'p', Description = "Path to the Game.p4k", EnvironmentVariable = "INPUT_P4K")]
    public required string P4kFile { get; init; }

    [CommandOption("output", 'o', Description = "Path to the output directory", EnvironmentVariable = "OUTPUT_FOLDER")]
    public required string OutputDirectory { get; init; }

    public ValueTask ExecuteAsync(IConsole console)
    {
        var p4k = P4k.P4kFile.FromFile(P4kFile);
        
        WriteFileForNode(OutputDirectory, P4kDirectoryNode.FromP4k(p4k));

        return default;
    }
    
    private static void WriteFileForNode(string baseDir, P4kDirectoryNode directoryNode)
    {
        var dir = new XElement("Directory",
            new XAttribute("Name", directoryNode.Name)
        );

        foreach (var (_, childNode) in directoryNode.Children.OrderBy(x => x.Key))
        {
            switch (childNode)
            {
                case P4kDirectoryNode childDirectoryNode:
                    //if we're a directory, Call ourselves recursively
                    WriteFileForNode(Path.Combine(baseDir, childDirectoryNode.Name), childDirectoryNode);
                    break;
                case P4kFileNode childFileNode:
                    dir.Add(new XElement("File",
                        new XAttribute("Name", Path.GetFileName(childFileNode.P4KEntry.Name)),
                        new XAttribute("CRC32", $"0x{childFileNode.P4KEntry.Crc32:X8}"),
                        //Revisit: they seem to change lastmodified a lot while the crc32 stays the same. I'll just ignore the date for now.
                        // new XAttribute("LastModified", childFileNode.ZipEntry.LastModified.ToString("O")),
                        new XAttribute("Size", childFileNode.P4KEntry.UncompressedSize.ToString(CultureInfo.InvariantCulture)),
                        //new XAttribute("CompressedSize", childFileNode.ZipEntry.CompressedSize.ToString(CultureInfo.InvariantCulture)),
                        new XAttribute("CompressionType", childFileNode.P4KEntry.CompressionMethod.ToString(CultureInfo.InvariantCulture)),
                        new XAttribute("Encrypted", childFileNode.P4KEntry.IsCrypted.ToString(CultureInfo.InvariantCulture))
                    ));
                    break;
                default:
                    throw new InvalidOperationException("Unknown node type");
            }
        }

        if (dir.HasElements)
        {
            var filePath = Path.Combine(baseDir, directoryNode.Name) + ".xml";
            Directory.CreateDirectory(Path.GetDirectoryName(filePath)!);
            dir.Save(filePath);
        }
    }
}