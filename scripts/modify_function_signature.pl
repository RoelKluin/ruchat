#!/usr/bin/perl
use strict;
use warnings;
use Getopt::Long;
use List::Util qw(uniq);

my $func;
my $full_arg;
my $default;
my $file;
my $position = 'end';
my @add_generics;
my @add_bounds;

GetOptions(
    "func=s" => \$func,
    "full-arg=s" => \$full_arg,
    "default=s" => \$default,
    "file=s" => \$file,
    "position=s" => \$position,
    "add-generic=s@" => \@add_generics,
    "add-bound=s@" => \@add_bounds,
);

die "Usage: $0 --func <function_name> --full-arg <full_arg_def> --default <default_expr> [--file <file_path>] [--position <start|end>] [--add-generic <generic>] [--add-bound <bound>]\n" unless $func && $full_arg && $default;

die "Invalid --position, must be 'start' or 'end'\n" unless $position eq 'start' || $position eq 'end';

# Parse arg_name and is_mut from full_arg
my $arg_name;
my $is_mut = 0;
if ($full_arg =~ /^\s*(mut\s+)?(\w+)\s*:/) {
    $is_mut = 1 if $1;
    $arg_name = $2;
} else {
    die "Invalid --full-arg format, cannot parse arg name\n";
}

# Extract new lifetimes from full_arg
my @new_lifetimes;
while ($full_arg =~ /'(\w+)/g) {
    push @new_lifetimes, $1;
}
@new_lifetimes = uniq sort @new_lifetimes;

# Find potential definition files
my @def_files = `rg --type rust -l "\\bfn\\s+\\Q$func\\E\\b\\s*\\("`;
chomp foreach @def_files;
my %unique_defs = map { $_ => 1 } @def_files;
@def_files = keys %unique_defs;

my $def_file;
if ($file) {
    if (exists $unique_defs{$file}) {
        $def_file = $file;
    } else {
        die "Specified file $file does not contain definition of $func\n";
    }
} else {
    if (@def_files == 0) {
        die "No definition found for $func\n";
    } elsif (@def_files > 1) {
        die "Multiple definitions found in files: @def_files\nPlease specify --file\n";
    } else {
        $def_file = $def_files[0];
    }
}

# Find all files containing the function name followed by '(' (calls and def)
my @files = `rg --type rust -l "\\b\\Q$func\\E\\b\\s*\\("`;
chomp foreach @files;
my %unique_files = map { $_ => 1 } @files;
@files = keys %unique_files;

# Modify each file
foreach my $f (@files) {
    local $/ = undef;
    open my $fh, '<', $f or die "Cannot open $f: $!";
    my $content = <$fh>;
    close $fh;

    # Modify the function definition if this is the def file
    if ($f eq $def_file) {
        $content =~ s{ ( (?:pub\s+|unsafe\s+|extern\s+(?:["'][^"']*["'])?\s+)? (?:async\s+)? fn \s+ \Q$func\E \b \s* ( [^(\n]* ) \s* \( \s* ( .*? ) \s* \) \s* ( -> \s* [^{]*? )? \s* ( where \s* [^{]*? )? \s* \{ ) }{
            my ($whole_sig, $generics_part, $params, $ret, $where) = ($1, $2, $3, $4, $5);

            # Process generics
            $generics_part =~ s/^\s+|\s+$//g;
            my $generics = '';
            if ($generics_part =~ /^<\s*(.*?)\s*>$/) {
                $generics = $1;
            }
            my @existing_lt;
            my @gen_items = ();
            if ($generics) {
                @gen_items = split /\s*,\s*/, $generics;
                foreach (@gen_items) {
                    if (/^\s*'(\w+)/) {
                        push @existing_lt, $1;
                    }
                }
            }
            my @add_lt = grep { my $lt = $_; !grep { $_ eq $lt } @existing_lt } @new_lifetimes;
            my @add_lt_items = map { "'$_" } @add_lt;

            # Insert new lifetimes before other generics
            my $insert_pos = 0;
            for (; $insert_pos < @gen_items; $insert_pos++) {
                last if $gen_items[$insert_pos] !~ /^\s*'\w+/;
            }
            splice @gen_items, $insert_pos, 0, @add_lt_items;

            # Add new generics
            push @gen_items, @add_generics if @add_generics;

            my $new_generics = join ', ', @gen_items;
            my $new_generics_str = $new_generics ? " <$new_generics> " : ' ';

            # Process params
            $params =~ s/^\s+|\s+$//g;
            my @param_list = $params ? split /\s*,\s*/, $params : ();
            if ($position eq 'end') {
                push @param_list, $full_arg;
            } else {
                unshift @param_list, $full_arg;
            }
            my $new_params = join ', ', @param_list;

            # Process where
            my $new_where = $where // '';
            if (@add_bounds) {
                my $bounds_str = join ', ', @add_bounds;
                if ($new_where) {
                    $new_where =~ s/(where\s*.*?)(\s*\{?)/$1, $bounds_str$2/s;
                } else {
                    $new_where = " where $bounds_str ";
                }
            }

            # Rebuild signature
            my $new_sig = $whole_sig =~ s/ (fn \s+ \Q$func\E \b \s* ) [^(\n]* ( \s* \( ) \s* .*? \s* ( \) ) \s* ( -> \s* [^{]*? )? \s* ( where \s* [^{]*? )? \s* ( \{ ) /$1$new_generics_str$2 $new_params $3$4 $new_where $6 /sr;
            $new_sig;
        }se;
    }

    # Modify the call sites
    my $let_mut = $is_mut ? ' mut' : '';
    $content =~ s{(?<!fn\s)\b\Q$func\E\b\s*\(\s*(.*?)\s*\)}{
        my $params = $1;
        my $new_call_params;
        if ($position eq 'end') {
            my $comma = ($params !~ /\S/) ? '' : ($params =~ /,\s*$/) ? '' : ', ';
            $new_call_params = $params . $comma . $arg_name;
        } else {
            my $comma = ($params =~ /\S/) ? ', ' : '';
            $new_call_params = $arg_name . $comma . $params;
        }
        "{ let$let_mut $arg_name = $default; \Q$func\E($new_call_params) }"
    }seg;

    # Write back the modified content
    open $fh, '>', $f or die "Cannot write to $f: $!";
    print $fh $content;
    close $fh;
}

# Test compilation
my $build_status = system('cargo build');
if ($build_status == 0) {
    my $commit_msg = "Add $full_arg argument to $func at $position and update call sites with default value";
    system('git add .');
    system("git commit -m \"$commit_msg\"");
} else {
    print "Compilation failed. Changes not committed.\n";
}
